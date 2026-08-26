use crate::background::BackgroundPattern;
use crate::font::{Font, FontFamily, FontSet, FontStyle};
use crate::geometry::{Color, Rect, Size};
use crate::theme::Theme;

/// Opaque clip-stack token returned by [`Painter::push_clip`]. Pass it back
/// to [`Painter::restore_clip`] to pop the clip the caller installed.
#[derive(Clone, Copy)]
pub struct SavedClip(Option<(i32, i32, i32, i32)>);

/// A widget's frame, in device pixels, ready to be measured in the *depths* its
/// design is written in.
///
/// Handed to a [`Painter::crisp`] recipe. Win 3.1 chrome is described as a set
/// of depths in logical pixels — a button's black outline occupies the first,
/// the bevel under it the next two — and this turns any such depth into device
/// pixels the way [`SvgImage`](crate::SvgImage) turns a viewBox coordinate into
/// one: scale the boundary, round *that*, and fill between the results. Nothing
/// accumulates,
/// because every boundary is measured from the widget's own edge and
/// neighbouring rings meet on the one they share, so a band lands within half a
/// device pixel of the depth it is drawn at however many rings deep it is.
///
/// Rounding each ring's *thickness* instead — one shared
/// [`Painter::chrome_unit`] per line — makes every ring the same weight, but
/// the error then compounds with depth and steps in whole units. A bevel two
/// logical pixels deep comes out four device pixels at 2.25x and six at 2.5x: a
/// 50% jump for an 11% change of scale, and 20% deeper than the five it is
/// drawn as. Depths give five at both, and six at 2.75x.
///
/// Below 1.5x the depths are the design's own instead of scaled ones, for the
/// reason [`Frame::new`] gives.
///
/// The price is that the rings *within* one band can differ by a device pixel —
/// at 2.5x a button's bevel is a ring 2 deep over a ring 3 deep. Nothing shows,
/// since both carry the same pair of colours and the band's total is what the
/// eye measures; and where a difference would show, it is at least
/// *consistent*, a ring being the same thickness on all four sides and the same
/// on every widget at that scale. Snapping each line from its own position in
/// the window, which is what the plain drawing path does, could promise
/// neither.
#[derive(Clone, Copy)]
pub struct Frame {
    /// The widget's rect, snapped to device pixels.
    rect: Rect,
    /// Device pixels one logical pixel of *depth* is worth: the window's scale,
    /// or 1.0 below 1.5x where the chrome keeps its drawn widths. Not the 1.0
    /// the painter itself runs at inside the pass.
    per_pixel: f32,
}

impl Frame {
    /// A frame over `rect` (already in device pixels) for a window drawn at
    /// `scale`.
    ///
    /// Below 1.5x the depths are pinned to the design's own: a logical pixel is
    /// still worth a single device pixel there — [`Painter::chrome_unit`] is 1 —
    /// and scaling the depths anyway only distorts the frame's proportions,
    /// since half a device pixel of rounding error is most of a 1-pixel line. At
    /// 1.25x it would put a 3-pixel bevel under a 1-pixel border where the
    /// design says 2 and 1. Keeping the drawn widths spends the extra room on
    /// the face instead: the button comes out a little roomier and a lot
    /// sharper.
    fn new(rect: Rect, scale: f32) -> Self {
        let per_pixel = if scale.round() > 1.0 { scale } else { 1.0 };
        Self { rect, per_pixel }
    }

    /// The frame's outer bounds, in device pixels.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Device pixels from the frame's edge in to logical depth `d`. `depth(0)`
    /// is the edge itself; `depth(1)` the far side of a one-pixel border.
    ///
    /// Scaled from the window's scale factor, except below 1.5x — see
    /// [`Frame::new`], where the reasoning lives.
    pub fn depth(&self, d: i32) -> i32 {
        (d as f32 * self.per_pixel).round().max(0.0) as i32
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
        let (ax, ay) = self.axis_depth(d);
        let (bx, by) = self.axis_depth(d + 1);
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

    /// A depth clamped to each axis of the frame, so a widget shallower than
    /// its own chrome overlaps itself rather than spilling over its neighbour.
    /// Everything else here goes through this.
    fn axis_depth(&self, d: i32) -> (i32, i32) {
        let depth = self.depth(d);
        (depth.min(self.rect.w.max(0)), depth.min(self.rect.h.max(0)))
    }
}

/// One ring of a [`Frame`]: its four sides at a single logical depth, each
/// already a device-pixel rect.
///
/// `top` / `left` are the pair a bevel lights and `bottom` / `right` the pair it
/// shadows, which is why [`Painter::fill_ring`] takes its two colours in that
/// order. A side is drawn with [`Painter::fill_rect`] like any other rect, so a
/// recipe that wants only one half of a ring — a scrollbar's highlight, with no
/// shadow opposite it — just paints the sides it wants.
#[derive(Clone, Copy)]
pub struct Ring {
    pub top: Rect,
    pub left: Rect,
    pub bottom: Rect,
    pub right: Rect,
}

impl Ring {
    /// The same ring with its corners left unpainted: every side pulled back at
    /// both ends by the thickness of the side it would have met there. The
    /// rounded outline a Win 3.1 button wears.
    pub fn cut_corners(self) -> Self {
        let (across, down) = (self.left.w, self.top.h);
        Self {
            top: Rect::new(
                self.top.x + across,
                self.top.y,
                self.top.w - 2 * across,
                self.top.h,
            ),
            bottom: Rect::new(
                self.bottom.x + across,
                self.bottom.y,
                self.bottom.w - 2 * across,
                self.bottom.h,
            ),
            left: Rect::new(
                self.left.x,
                self.left.y + down,
                self.left.w,
                self.left.h - 2 * down,
            ),
            right: Rect::new(
                self.right.x,
                self.right.y + down,
                self.right.w,
                self.right.h - 2 * down,
            ),
        }
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
/// device pixels off a [`Frame`], which places each ring by scaling the depth
/// the design puts it at, the way an [`SvgImage`](crate::SvgImage) is
/// rasterized. See [`Painter::crisp`].
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
    /// display. On the Wayland backend `scale` is the integer buffer scale the
    /// content is rasterized at (2.0 for a 150% display) and the compositor
    /// resamples 2.0→1.5; `system_scale` records the real 1.5 so UI can report
    /// it. Equal to `scale` on every other backend and until a backend calls
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
    /// popup top-level window whose [`PopupRequest::rect`] is `anchor`.
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
        }
    }

    /// Record where the screen is, for widgets that place something on it —
    /// see [`Self::screen_area`]. Only the runtime has the answer, so only the
    /// runtime calls this; an offscreen render leaves it unset.
    pub fn with_screen(mut self, area: Option<Rect>) -> Self {
        self.screen = area;
        self
    }

    pub fn is_popup_pass(&self) -> bool {
        self.popup_anchor.is_some()
    }

    /// [`PopupRequest::rect`] of the popup this painter is drawing into,
    /// or `None` in the main pass. Widgets that report a
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
        let prev = SavedClip(self.clip);
        let x0 = self.origin_x + self.snap(rect.x);
        let y0 = self.origin_y + self.snap(rect.y);
        let x1 = self.origin_x + self.snap(rect.x + rect.w);
        let y1 = self.origin_y + self.snap(rect.y + rect.h);
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
    /// display). On the Wayland backend this can differ from [`Self::scale`],
    /// the integer buffer scale the content is actually rasterized at before
    /// the compositor resamples it down to the fractional size; on every other
    /// backend the two are equal. Defaults to [`Self::scale`] until a backend
    /// calls [`Self::set_system_scale`].
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
    /// winit runtime, whose scale carries saudade's density correction on top
    /// of the factor the display reports.
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

    /// Device pixels one logical pixel of chrome is worth on its own: the scale
    /// rounded to a whole pixel, never less than one. The same `unit` the window
    /// chrome in [`crate::chrome`] draws its frame lines at, and what a widget
    /// wants for a *lone* thin line — a divider, a caret, a grid rule.
    ///
    /// Not the way to build a frame. Multiplying this by a depth rounds the
    /// wrong quantity: the error compounds with every ring and steps in whole
    /// units, so a two-pixel bevel jumps from four device pixels to six between
    /// 2.25x and 2.5x. Ask a [`Frame`] for the ring instead — it rounds the
    /// depth, which is [`Frame::depth(1)`](Frame::depth) for a single line and
    /// stays exact however deep the chrome goes.
    pub fn chrome_unit(&self) -> i32 {
        (self.scale.round() as i32).max(1)
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
    /// [`Frame`] that places its rings by scaling the design's depths.
    ///
    /// The painter is dropped to device pixels exactly as [`Self::physical`]
    /// hands it over, and the `Frame` carries `rect` snapped to its device
    /// bounds together with the *outer* scale. A recipe asks the frame for the
    /// ring it wants to paint — `f.ring(0)` for the outline, `f.ring(1)` and
    /// `f.ring(2)` for the bevel behind it, `f.inside(3)` for the face left
    /// over — and every one of those comes back with its boundaries already
    /// rounded to device pixels. Because the outer rect is the snapped
    /// one, the frame still lands exactly where the scaled draw would have put
    /// it, and still shares its edge pixels with the widget next door.
    ///
    /// At 1.0x, and at any integer scale where every depth is an exact multiple,
    /// this paints the same pixels the plain logical path would. It diverges
    /// only where that path was snapping each line from its own position in the
    /// window and so came out uneven — heavier along one edge of a frame than
    /// the opposite one, and different again on the widget beside it.
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
        let scale = self.scale;
        self.physical(rect, |p, r| f(p, Frame::new(r, scale)));
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
    /// draws straight into `area` (no allocation, via [`Self::with_scale`]);
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
        let thick = self.scale.round().max(1.0) as i32;
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
        let step = self.scale.round().max(1.0) as i32;
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
        let inv = 255 - a;
        let sr = color.red() as u32;
        let sg = color.green() as u32;
        let sb = color.blue() as u32;
        let dr = (dst >> 16) & 0xFF;
        let dg = (dst >> 8) & 0xFF;
        let db = dst & 0xFF;
        let r = (sr * a + dr * inv) / 255;
        let g = (sg * a + dg * inv) / 255;
        let b = (sb * a + db * inv) / 255;
        self.pixels[idx] = 0xFF000000 | (r << 16) | (g << 8) | b;
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
    /// The far sides paint last, so they own the two corners they share with
    /// the near ones — the diagonal a Win 3.1 bevel runs along.
    pub fn fill_ring(&mut self, ring: Ring, near: Color, far: Color) {
        self.fill_rect(ring.top, near);
        self.fill_rect(ring.left, near);
        self.fill_rect(ring.bottom, far);
        self.fill_rect(ring.right, far);
    }

    pub fn stroke_rect(&mut self, rect: Rect, color: Color) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        self.crisp(rect, |p, f| p.fill_ring(f.ring(0), color, color));
    }

    /// Raised 3D bevel: light highlight on top/left, dark shadow on bottom/right.
    pub fn raised_bevel(&mut self, rect: Rect, highlight: Color, shadow: Color) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        self.crisp(rect, |p, f| p.fill_ring(f.ring(0), highlight, shadow));
    }

    pub fn sunken_bevel(&mut self, rect: Rect, highlight: Color, shadow: Color) {
        self.raised_bevel(rect, shadow, highlight);
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
    /// with rounded (unpainted) corners; the default button gets an
    /// additional sharp-cornered outer border. Light-gray face, raised
    /// bevel, sunken when pressed.
    ///
    /// Drawn in one [`Self::crisp`] pass, with every ring placed at its scaled
    /// depth. That is what keeps the bevel as deep as it is drawn — 5 device
    /// pixels at 2.5x, where rounding each of its two rings to a whole 3 and
    /// adding them gave 6.
    pub fn button(&mut self, rect: Rect, theme: &Theme, pressed: bool, default: bool) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        self.crisp(rect, |p, f| {
            // Rounded black outline, then the default button's extra ring —
            // square-cornered, and the same black, so the two read as one
            // heavier border whose depth is still exactly `depth(2)`.
            p.fill_ring(f.ring(0).cut_corners(), theme.border, theme.border);
            let face = if default {
                p.fill_ring(f.ring(1), theme.border, theme.border);
                2
            } else {
                1
            };
            p.fill_rect(f.inside(face), theme.face);
            // The bevel is two logical pixels deep: two rings, meeting on a
            // shared depth, so together they span exactly `depth(face + 2) -
            // depth(face)` and still step at the corners.
            let (outer, inner) = (f.ring(face), f.ring(face + 1));
            if pressed {
                // Sunken: the shadow takes both pixels along the top/left, and
                // a single highlight line sits opposite it.
                p.fill_ring(outer, theme.shadow, theme.highlight);
                p.fill_rect(inner.top, theme.shadow);
                p.fill_rect(inner.left, theme.shadow);
            } else {
                p.fill_ring(outer, theme.highlight, theme.shadow);
                p.fill_ring(inner, theme.highlight, theme.shadow);
            }
        });
    }

    /// Lighter Win 3.1 chrome used by the scrollbar's arrow buttons and thumb:
    /// a square (un-rounded) 1px black outline, a single highlight line on the
    /// top/left, and a 2px shadow on the bottom/right. Reads as lighter than
    /// [`Self::button`], whose rounded outline and doubled highlight give the
    /// heavier "dialog" chrome.
    ///
    /// When `pressed`, the face is drawn pushed in: the top/left carries a
    /// single shadow line (no highlight) and the bottom/right loses its shadow
    /// — the inverse of the raised look, the way a held scrollbar arrow sinks
    /// in Win 3.1.
    pub fn light_button(&mut self, rect: Rect, theme: &Theme, pressed: bool) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        self.crisp(rect, |p, f| {
            p.fill_ring(f.ring(0), theme.border, theme.border);
            p.fill_rect(f.inside(1), theme.face);
            let inner = f.ring(1);
            if pressed {
                // Depressed: a single shadow line on the top/left, no highlight
                // and no bottom/right shadow, so the button reads as pushed in.
                p.fill_rect(inner.top, theme.shadow);
                p.fill_rect(inner.left, theme.shadow);
                return;
            }
            // One highlight line on the top/left...
            p.fill_rect(inner.top, theme.highlight);
            p.fill_rect(inner.left, theme.highlight);
            // ...against a 2px shadow on the bottom/right: two rings sharing a
            // depth, so the pair is exactly `depth(3) - depth(1)` deep.
            p.fill_rect(inner.bottom, theme.shadow);
            p.fill_rect(inner.right, theme.shadow);
            let deeper = f.ring(2);
            p.fill_rect(deeper.bottom, theme.shadow);
            p.fill_rect(deeper.right, theme.shadow);
        });
    }

    /// Dotted Win 3.1 focus rectangle: a dashed outline — one dot, one gap —
    /// tracing the edges of `rect`.
    ///
    /// The one piece of chrome that wants a uniform weight rather than exact
    /// depths, because what the eye reads in a dash is its rhythm. The dot is
    /// [`Frame::depth(1)`](Frame::depth) square and the pitch twice that, so the
    /// ring is evenly dotted at every scale; placing each dot at its exact
    /// scaled position instead would put 2- and 3-pixel gaps between 2-pixel
    /// dots at 2.25x, which reads as a smear and not as dots. The cost is a
    /// pitch up to a pixel off nominal, which a dash has no geometry riding on.
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
        let Some(font) = self.fonts.sans else {
            return;
        };
        let (w, h) = font.measure(text, size);
        let tx = rect.x + ((rect.w as f32 - w) / 2.0).round() as i32;
        let ty = rect.y + ((rect.h as f32 - h) / 2.0).round() as i32;
        self.text(tx, ty, text, size, color);
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
        let thick = scale.round().max(1.0) as i32;
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

    /// Every scale a saudade window is plausibly drawn at: the ladder Windows
    /// and X11 hand over, plus the quarter steps a density-corrected Mac snaps
    /// to (see [`crate::density`]). The frame primitives have to hold their
    /// geometry on all of them, in order, which is what a [`Frame`]'s depths
    /// are for.
    const CHROME_SCALES: &[f32] = &[1.0, 1.25, 1.5, 2.0, 2.25, 2.5, 2.75, 3.0];

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
    /// including the pin to the design's own widths below 1.5x.
    fn depth(d: i32, scale: f32) -> i32 {
        let per_pixel = if scale.round() > 1.0 { scale } else { 1.0 };
        (d as f32 * per_pixel).round() as i32
    }

    #[test]
    fn a_chrome_unit_is_the_scale_rounded_to_a_whole_pixel() {
        let expected = [
            (1.0, 1),
            (1.25, 1),
            (1.5, 2),
            (2.0, 2),
            (2.25, 2),
            (2.5, 3),
            (2.75, 3),
            (3.0, 3),
        ];
        for (scale, unit) in expected {
            let mut pixels = [0u32; 4];
            let p = Painter::new(&mut pixels, 2, 2, scale, 0, 0, FontSet::default());
            assert_eq!(p.chrome_unit(), unit, "the unit at {scale}x");
        }
    }

    /// A section through a plain button, at every scale: the black outline runs
    /// to `depth(1)` and the bevel from there to `depth(3)`, on all four sides.
    /// Both bands are measured from the button's edge, so neither carries the
    /// other's rounding error, and the chrome is exactly as deep as the three
    /// logical pixels it is drawn as.
    #[test]
    fn a_button_frames_bands_land_on_their_scaled_depths() {
        let theme = Theme::windows_31();
        for &scale in CHROME_SCALES {
            let (border, chrome) = (depth(1, scale), depth(3, scale));
            let (w, h) = phys(60, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.button(Rect::new(0, 0, 60, 30), &theme, false, false)
            });
            let expected = |across: i32| {
                vec![
                    (theme.border.0, border),
                    (theme.highlight.0, chrome - border),
                    (theme.face.0, across - 2 * chrome),
                    (theme.shadow.0, chrome - border),
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

    /// Up to 1.25x — up to 1.5x, where a logical pixel stops being worth a
    /// single device one — the frame keeps the widths it is drawn at, and the
    /// room the scale buys goes to the face instead. A button at 1.25x is a
    /// quarter bigger around the same 1-pixel border and 2-pixel bevel: roomier,
    /// and far sharper than the 3-pixel bevel under a 1-pixel border that
    /// scaling the depths there would give.
    #[test]
    fn below_one_and_a_half_a_frame_keeps_its_drawn_widths() {
        let theme = Theme::windows_31();
        for &scale in &[1.0f32, 1.1, 1.25, 1.4] {
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
        // From 1.5x the depths scale, and the border thickens with them.
        let (w, h) = phys(60, 30, 1.5);
        let px = chrome_buffer(w, h, 1.5, |p| {
            p.button(Rect::new(0, 0, 60, 30), &theme, false, false)
        });
        assert_eq!(
            column(&px, w, h, w / 2)[0],
            (theme.border.0, 2),
            "the border at 1.5x, where depths start scaling"
        );
    }

    /// The default button's extra ring is the same black directly inside the
    /// outline, so the border reads as one band two logical pixels deep — and
    /// lands on `depth(2)` exactly, not on twice whatever the first ring
    /// rounded to.
    #[test]
    fn a_default_buttons_doubled_border_lands_on_depth_two() {
        let theme = Theme::windows_31();
        for &scale in CHROME_SCALES {
            let (border, chrome) = (depth(2, scale), depth(4, scale));
            let (w, h) = phys(60, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.button(Rect::new(0, 0, 60, 30), &theme, false, true)
            });
            assert_eq!(
                column(&px, w, h, w / 2),
                vec![
                    (theme.border.0, border),
                    (theme.highlight.0, chrome - border),
                    (theme.face.0, h - 2 * chrome),
                    (theme.shadow.0, chrome - border),
                    (theme.border.0, border),
                ],
                "vertical section of a default button at {scale}x"
            );
        }
    }

    /// Pressed, the bevel inverts: the shadow takes both logical pixels on the
    /// top/left and a single highlight line sits opposite it.
    #[test]
    fn a_pressed_button_inverts_its_bevel() {
        let theme = Theme::windows_31();
        for &scale in CHROME_SCALES {
            let (one, two, three) = (depth(1, scale), depth(2, scale), depth(3, scale));
            let (w, h) = phys(60, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.button(Rect::new(0, 0, 60, 30), &theme, true, false)
            });
            assert_eq!(
                column(&px, w, h, w / 2),
                vec![
                    (theme.border.0, one),
                    (theme.shadow.0, three - one),
                    (theme.face.0, h - three - two),
                    (theme.highlight.0, two - one),
                    (theme.border.0, one),
                ],
                "vertical section of a pressed button at {scale}x"
            );
        }
    }

    /// The scrollbar's lighter chrome: outline, one highlight line, and the
    /// doubled shadow opposite — the band that used to double in width from one
    /// quarter step to the next, and now runs from `depth(1)` to `depth(3)`.
    #[test]
    fn a_light_buttons_doubled_shadow_lands_on_depth_three() {
        let theme = Theme::windows_31();
        for &scale in CHROME_SCALES {
            let (one, two, three) = (depth(1, scale), depth(2, scale), depth(3, scale));
            let (w, h) = phys(30, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.light_button(Rect::new(0, 0, 30, 30), &theme, false)
            });
            assert_eq!(
                column(&px, w, h, w / 2),
                vec![
                    (theme.border.0, one),
                    (theme.highlight.0, two - one),
                    (theme.face.0, h - two - three),
                    (theme.shadow.0, three - one),
                    (theme.border.0, one),
                ],
                "vertical section of a light button at {scale}x"
            );
        }
    }

    /// The point of measuring depths rather than multiplying a rounded
    /// thickness: a 2-logical-pixel bevel stays within a device pixel of the
    /// `2 × scale` it is drawn as, and never gains more than one pixel from one
    /// rung of the ladder to the next. Rounding each of its two rings and adding
    /// them gave 4 device pixels at 2.25x and 6 at 2.5x — a 50% jump for an 11%
    /// change of scale, and the reason this test exists.
    #[test]
    fn a_bevel_deepens_by_at_most_a_pixel_from_one_scale_to_the_next() {
        let theme = Theme::windows_31();
        let mut previous: Option<(f32, i32)> = None;
        for &scale in CHROME_SCALES {
            let (w, h) = phys(30, 30, scale);
            let px = chrome_buffer(w, h, scale, |p| {
                p.light_button(Rect::new(0, 0, 30, 30), &theme, false)
            });
            // The doubled shadow: the fourth band down the middle column.
            let bands = column(&px, w, h, w / 2);
            let (color, shadow) = bands[3];
            assert_eq!(color, theme.shadow.0, "the shadow band at {scale}x");

            let nominal = 2.0 * scale;
            assert!(
                (shadow as f32 - nominal).abs() <= 1.0,
                "a 2px bevel is {shadow} device px at {scale}x, nominal {nominal}"
            );
            if let Some((prev_scale, prev)) = previous {
                assert!(
                    (shadow - prev).abs() <= 1,
                    "the bevel jumps from {prev} to {shadow} device px \
                     between {prev_scale}x and {scale}x"
                );
            }
            previous = Some((scale, shadow));
        }
    }

    /// A dot, a gap, a dot — every one of them the same size, starting from the
    /// lit corner. A dash is the one piece of chrome placed by weight rather
    /// than by depth: what the eye reads here is the rhythm, and dots spaced at
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
    /// takes the other's rounding: one device pixel apiece up to 1.25x, and 3
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

    /// A widget shallower than the chrome it wears overlaps itself rather than
    /// spilling over its neighbour: a 1-logical-pixel rule at 2.75x is three
    /// device pixels of buffer and its outline is capped there, so the rule
    /// fills solid and stops.
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
}
