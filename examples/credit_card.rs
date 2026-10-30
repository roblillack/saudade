//! credit_card — a bank card at its real size, whatever the display is set to.
//!
//! This window draws an ISO/IEC 7810 ID-1 card — the die every bank card,
//! driving licence and door badge is cut to, 85.60 x 53.98 mm with a 3.18 mm
//! corner radius — at the size it is in your wallet. Take one out and lay it on
//! the screen: the two should be the same object.
//!
//! A logical pixel can't promise that. saudade aims one at 1/72 in (see
//! `src/ui_scale.rs`, and "How big a logical pixel is" in the README), but the
//! scale factor it gets there by is the display's density and the scaling the
//! user picked for it — 150% in Windows' Display Settings, "Larger Text" on a
//! Mac — multiplied together, and on a Mac it says nothing of the density at
//! all. So the card is drawn in device pixels instead, as many to the inch as
//! [`Painter::display_dpi`] says the display has, going by the physical size it
//! reports for itself. Change the scaling and the window around the card grows
//! or shrinks; the card stays the size it is. `SAUDADE_UI_SCALE` moves the
//! window the same way, and leaves the card alone too.
//!
//! The readout under the card adds how big a logical pixel came out on this
//! display, next to the 1/72 in it is aimed at — the check this window was
//! first written to make, and still the one way to see how far the nominal
//! scale misses a given display.
//!
//! Where the display reports no size — a projector or virtual output with none
//! to report — the card falls back to the nominal 72 logical pixels to the
//! inch, and the readout says so. That is the one case where `SAUDADE_UI_SCALE`
//! sizes the card, and the readout suggests the scales either side to try. A
//! display that misreports its size is taken at its word: the card is exactly
//! as right as the display's EDID.
//!
//! The macOS and Windows runtimes read that EDID figure off the platform, X11
//! off RandR, and Wayland off `wl_output`'s physical size and mode. Note what a
//! device pixel is: the one the painter draws in, which is the panel's wherever
//! `wp_viewporter` lets the buffer be the fractional size the compositor
//! presents. Where it doesn't, the buffer is oversampled at the integer scale
//! above and resampled down, so the density is the buffer's — 252 dpi for a
//! 157-dpi panel presented at 125% — and the card is counted in that instead.
//! Either way it comes out 85.60 mm on the glass; only the number of pixels
//! that takes changes.
//!
//! The window opens at the nominal size, there being nothing to know about a
//! display before the window is on one, and fits itself to the card on its
//! first paint — and again whenever it lands on a display of another density,
//! or comes back to the front after the scaling was changed under it.
//!
//! The face is a flat-painter exercise. The rounded corners are rasterized as
//! horizontal spans inside `Painter::physical`, in *device* pixels — an arc
//! stepped in logical pixels would jump two and two-thirds device pixels at a
//! time on a Retina Mac — the embossed lettering is a dark copy of the text
//! offset under a light one, and the two-disc mark paints its overlap in a third
//! colour because the painter has no alpha. Every feature is placed in
//! millimetres over the card's own face, the way it is on the real thing.
//!
//! ```console
//! $ cargo run --example credit_card
//! $ SAUDADE_UI_SCALE=4 cargo run --example credit_card  # bigger window, same card
//! ```

use saudade::{
    App, Color, Event, EventCtx, FontFamily, FontStyle, Label, Painter, Rect, Size, Theme, Widget,
    WindowConfig,
};

// ---------------------------------------------------------------------------
// The card, in millimetres
// ---------------------------------------------------------------------------

/// ISO/IEC 7810 ID-1, the bank-card die: 85.60 x 53.98 mm, corners on a 3.18 mm
/// radius. The tolerance the standard allows is ±0.13 mm, a device pixel or so
/// on the densest panel, so a card that misses this drawing misses it because
/// of the display's figures and not because of the card.
const CARD_W_MM: f32 = 85.60;
const CARD_H_MM: f32 = 53.98;
const CORNER_MM: f32 = 3.18;

/// Logical pixels to an inch: the 72 saudade aims one at (see
/// `src/ui_scale.rs`), which is all the card has to go on where the display
/// won't say how big it is.
const NOMINAL_DPI: f32 = 72.0;
const MM_PER_IN: f32 = 25.4;

/// Millimetres to logical pixels at the nominal density — 85.60 mm comes out
/// 243, 53.98 mm 153.
fn nominal_px(mm: f32) -> i32 {
    (mm * NOMINAL_DPI / MM_PER_IN).round() as i32
}

/// The account number, for once not arbitrary: 8560 5398 is the card's own
/// size in hundredths of a millimetre, 0318 its corner radius and 0076 its
/// thickness.
const NUMBER: &str = "8560 5398 0318 0076";
const CARDHOLDER: &str = "LOGICAL PIXEL";
const EXPIRY: &str = "12/99";

// The palette. Nothing here is a theme colour: the card is an object sitting on
// the desktop, not a piece of window chrome.
const EDGE: Color = Color::rgb(0x08, 0x10, 0x28);
const BAND: Color = Color::rgb(0x2A, 0x4C, 0x96);
const FACE: Color = Color::rgb(0x18, 0x31, 0x6E);
const CHIP: Color = Color::rgb(0xC8, 0xA4, 0x38);
const CHIP_LINE: Color = Color::rgb(0x6C, 0x54, 0x10);
const PRINT: Color = Color::rgb(0xE4, 0xE8, 0xF0);
const PRINT_FAINT: Color = Color::rgb(0x9C, 0xB0, 0xD8);
const RAISED: Color = Color::rgb(0xE8, 0xEC, 0xF4);
const RAISED_SHADOW: Color = Color::rgb(0x0C, 0x1C, 0x40);
const DISC_LEFT: Color = Color::rgb(0xCC, 0x33, 0x33);
const DISC_RIGHT: Color = Color::rgb(0xE8, 0xA8, 0x20);
const DISC_OVERLAP: Color = Color::rgb(0xD8, 0x66, 0x20);

// ---------------------------------------------------------------------------
// The window, in logical pixels
// ---------------------------------------------------------------------------

const MARGIN: i32 = 16;
/// Line height of the default 11-pixel theme font, as `Label` computes it.
const LINE: i32 = 14;

fn main() {
    let (config, root) = window();
    App::new(config, root).with_theme(Theme::windows_31()).run();
}

/// The window and the widget tree inside it, kept apart from `main` so
/// `examples/screenshots.rs` can render the same window offscreen.
pub fn window() -> (WindowConfig, impl Widget) {
    let sheet = Sheet::new();
    let size = sheet.size;
    (WindowConfig::new("Credit Card", size.w, size.h), sheet)
}

/// Everything in the window — the instruction, the card, the readout — laid
/// out around the card. How big that is only a paint can tell, the painter
/// being what knows the display, so a paint lays it out and the tick after it
/// asks for the window to match, which a paint has no way to.
struct Sheet {
    bounds: Rect,
    intro: Label,
    card: Card,
    readout: Readout,
    /// The window size the layout calls for, as of the last paint.
    size: Size,
    /// A size the last paint called for that the window has not been asked
    /// for yet.
    pending: Option<Size>,
}

impl Sheet {
    /// Laid out for the nominal card, which is what there is to go on before
    /// the window has been painted.
    fn new() -> Self {
        let unplaced = Rect::new(0, 0, 1, 1);
        let mut sheet = Self {
            bounds: unplaced,
            intro: Label::new(
                unplaced,
                "Lay a bank card on the screen, over the one below: they should be \
                 the same size.",
            ),
            card: Card {
                rect: unplaced,
                size: Size::new(1, 1),
            },
            readout: Readout {
                rect: unplaced,
                lines: Default::default(),
            },
            size: Size::new(1, 1),
            pending: None,
        };
        let card = Size::new(nominal_px(CARD_W_MM), nominal_px(CARD_H_MM));
        sheet.size = sheet.arrange(card, 0);
        sheet.bounds = Rect::new(0, 0, sheet.size.w, sheet.size.h);
        sheet
    }

    /// Place everything around a card `card` logical pixels big, under text
    /// `text_w` wide, and return the window size that adds up to.
    fn arrange(&mut self, card: Size, text_w: i32) -> Size {
        let inner = card.w.max(text_w);
        // Three lines of box for two lines of text: the host's sans face decides
        // how far the instruction wraps, and a `Label` clips what doesn't fit.
        self.intro.rect = Rect::new(MARGIN, 12, inner, 3 * LINE);
        self.card.rect = Rect::new(
            MARGIN + (inner - card.w) / 2,
            self.intro.rect.bottom() + 6,
            card.w,
            card.h,
        );
        self.readout.rect = Rect::new(MARGIN, self.card.rect.bottom() + 12, inner, 3 * LINE);
        Size::new(inner + 2 * MARGIN, self.readout.rect.bottom() + 10)
    }
}

impl Widget for Sheet {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn layout(&mut self, bounds: Rect) {
        self.bounds = bounds;
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let fit = Fit::of(painter);
        let lines = fit.readout();
        let text_w = lines
            .iter()
            .map(|line| painter.measure_text(line, theme.font_size).w)
            .max()
            .unwrap_or(0);
        // Compared against the last size asked for rather than the window's, so
        // a window the user has resized since stays the size they made it.
        let size = self.arrange(fit.logical(), text_w);
        if size != self.size {
            self.size = size;
            self.pending = Some(size);
        }
        self.card.size = fit.card;
        self.readout.lines = lines;

        self.intro.paint(painter, theme);
        self.card.paint(painter, theme);
        self.readout.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if matches!(event, Event::Tick)
            && let Some(size) = self.pending.take()
        {
            ctx.request_window_size(size.w, size.h);
        }
    }

    fn wants_ticks(&self) -> bool {
        self.pending.is_some()
    }
}

// ---------------------------------------------------------------------------
// How the card lands on this display
// ---------------------------------------------------------------------------

/// The card's size in device pixels on the display the window is on, and the
/// numbers that came from.
struct Fit {
    /// The card, in device pixels.
    card: Size,
    /// How many device pixels the display says it fits to the inch, or `None`
    /// if it says nothing and the card is at the nominal size.
    dpi: Option<f32>,
    /// The scale the window is drawn at, and the factor the display reports.
    scale: f32,
    system: f32,
}

impl Fit {
    fn of(painter: &Painter) -> Self {
        let scale = painter.scale();
        let dpi = painter.display_dpi();
        // Device pixels to the millimetre: the display's own, or the nominal
        // logical ones carried through the scale.
        let per_mm = dpi.unwrap_or(NOMINAL_DPI * scale) / MM_PER_IN;
        let px = |mm: f32| (mm * per_mm).round() as i32;
        Self {
            card: Size::new(px(CARD_W_MM), px(CARD_H_MM)),
            dpi,
            scale,
            system: painter.system_scale(),
        }
    }

    /// The room the card takes in logical pixels, rounded out so the device
    /// pixels it is drawn in always fit inside.
    fn logical(&self) -> Size {
        let room = |px: i32| (px as f32 / self.scale).ceil() as i32;
        Size::new(room(self.card.w), room(self.card.h))
    }

    /// What the card measures and what that came from; and either how big a
    /// logical pixel came out, or — with nothing measured — the scales to try
    /// when the card on the glass says this one is wrong.
    fn readout(&self) -> [String; 3] {
        let card = format!("{CARD_W_MM:.2} x {CARD_H_MM:.2} mm");
        let scales = format!(
            "{}x (display reports {}x)",
            trim(self.scale),
            trim(self.system)
        );
        let Some(dpi) = self.dpi else {
            // One rung either way — a twelfth, the grid the runtime snaps to,
            // and about 2 mm on the long edge of the card at 3x. The variable
            // sets the drawn scale outright, so these go in as they read.
            let rung = 1.0 / 12.0;
            let logical = self.logical();
            return [
                format!("{card} = {} x {} logical px", logical.w, logical.h),
                format!("{NOMINAL_DPI:.0} px/in assumed, at {scales}"),
                format!(
                    "off? try SAUDADE_UI_SCALE={} or {}",
                    trim((self.scale - rung).max(rung)),
                    trim(self.scale + rung)
                ),
            ];
        };
        [
            format!(
                "{card} = {} x {} px at {dpi:.0} px/in",
                self.card.w, self.card.h
            ),
            format!("drawn at {scales}"),
            format!(
                "so a logical pixel is 1/{:.0} in, aimed at 1/{NOMINAL_DPI:.0}",
                dpi / self.scale
            ),
        ]
    }
}

// ---------------------------------------------------------------------------
// The card
// ---------------------------------------------------------------------------

/// The card, drawn `size` device pixels big from the top-left corner of the
/// logical room laid out for it.
struct Card {
    rect: Rect,
    size: Size,
}

impl Widget for Card {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, _theme: &Theme) {
        // The embossed characters want the fixed-width face a real card's
        // are stamped in; a host that ships no monospace font gets the sans.
        let stamped = if painter.fonts().mono.is_some() {
            FontFamily::Mono
        } else {
            FontFamily::Sans
        };

        // Everything below is drawn in device pixels: the card is a physical
        // object, and neither its edges nor its arcs and hairlines have any
        // reason to round to the logical grid on the way there.
        let size = self.size;
        painter.physical(self.rect, |p, room| {
            let face = Face::new(Rect::new(room.x, room.y, size.w, size.h));
            paint_body(p, &face);
            paint_chip(p, &face);
            paint_lettering(p, &face, stamped);
            paint_mark(p, &face);
        });
    }
}

/// The card face as a coordinate system in millimetres over the device pixels
/// it was actually drawn on. The pixels-per-millimetre comes from the rectangle
/// drawn rather than from the display's density, so every feature lands inside
/// the card that really got drawn, however its edges rounded.
struct Face {
    rect: Rect,
    ppm: f32,
}

impl Face {
    fn new(rect: Rect) -> Self {
        Self {
            rect,
            ppm: rect.w as f32 / CARD_W_MM,
        }
    }

    fn x(&self, mm: f32) -> i32 {
        self.rect.x + (mm * self.ppm).round() as i32
    }

    fn y(&self, mm: f32) -> i32 {
        self.rect.y + (mm * self.ppm).round() as i32
    }

    /// A length in device pixels, never less than one — a hairline stays a
    /// hairline rather than disappearing on a 1x display.
    fn len(&self, mm: f32) -> i32 {
        ((mm * self.ppm).round() as i32).max(1)
    }

    fn area(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(self.x(x), self.y(y), self.len(w), self.len(h))
    }

    /// An em of `mm` millimetres, in the device pixels `Painter::text_styled`
    /// sizes text in while the painter sits at device scale.
    fn em(&self, mm: f32) -> f32 {
        mm * self.ppm
    }
}

/// The plastic: a hairline edge, a lighter band across the top quarter, the
/// body below it.
fn paint_body(p: &mut Painter, f: &Face) {
    let radius = f.len(CORNER_MM);
    let edge = f.len(0.25);

    rounded_rows(p, f.rect, radius, 0, f.rect.h, EDGE);

    let inner = f.rect.inset(edge);
    let inner_radius = (radius - edge).max(1);
    rounded_rows(p, inner, inner_radius, 0, inner.h, BAND);
    rounded_rows(p, inner, inner_radius, f.len(14.0) - edge, inner.h, FACE);
}

/// Fill rows `y0..y1` of a rounded rectangle, the rows counted from `r.y`.
/// Splitting the fill by row band is what lets the body take two tones without
/// either of them squaring off the corners.
///
/// No antialiasing, deliberately: the edge being compared against a real card
/// should be a hard one, and a Win 3.1 painter has no alpha to soften it with.
fn rounded_rows(p: &mut Painter, r: Rect, radius: i32, y0: i32, y1: i32, color: Color) {
    for row in y0.max(0)..y1.min(r.h) {
        let inset = corner_inset(row, r.h, radius);
        p.fill_rect(Rect::new(r.x + inset, r.y + row, r.w - 2 * inset, 1), color);
    }
}

/// How far in from either side the outline sits on `row`: nothing down the
/// straight middle, the arc's own offset within the two corner bands. Measured
/// from the row's centre, so the widest row of a corner is the arc's own end
/// and not half a pixel past it.
fn corner_inset(row: i32, height: i32, radius: i32) -> i32 {
    let r = radius as f32;
    let dy = if row < radius {
        r - (row as f32 + 0.5)
    } else if row >= height - radius {
        (row as f32 + 0.5) - (height - radius) as f32
    } else {
        return 0;
    };
    (r - (r * r - dy * dy).max(0.0).sqrt()).round() as i32
}

/// The contact plate, six pads on a 12 x 9.5 mm module.
fn paint_chip(p: &mut Painter, f: &Face) {
    let chip = f.area(8.0, 16.0, 12.0, 9.5);
    let radius = f.len(1.0);
    let line = f.len(0.25);

    rounded_rows(p, chip, radius, 0, chip.h, CHIP_LINE);
    let pads = chip.inset(line);
    rounded_rows(p, pads, (radius - line).max(1), 0, pads.h, CHIP);

    p.fill_rect(
        Rect::new(chip.x, chip.y + (chip.h - line) / 2, chip.w, line),
        CHIP_LINE,
    );
    for third in 1..=2 {
        p.fill_rect(
            Rect::new(chip.x + chip.w * third / 3, chip.y, line, chip.h),
            CHIP_LINE,
        );
    }
}

/// Printed marks and embossed ones. The printed ones are flat ink; the embossed
/// ones are stamped through the plastic, which a flat painter can say with a
/// dark copy of the glyphs offset under a light one.
fn paint_lettering(p: &mut Painter, f: &Face, stamped: FontFamily) {
    p.text_styled(
        f.x(7.5),
        f.y(4.6),
        "SAUDADE",
        f.em(4.8),
        PRINT,
        FontFamily::Sans,
        FontStyle::Bold,
    );
    p.text_styled(
        f.x(7.7),
        f.y(10.6),
        "ID-1 REFERENCE CARD",
        f.em(2.7),
        PRINT_FAINT,
        FontFamily::Sans,
        FontStyle::Regular,
    );
    p.text_styled(
        f.x(8.0),
        f.y(40.2),
        "VALID THRU",
        f.em(2.7),
        PRINT_FAINT,
        FontFamily::Sans,
        FontStyle::Regular,
    );

    // The number spans most of the card, and the monospace face is whatever the
    // host ships — measure it and pull the size in if that one runs wide.
    let number_em = fit(p, NUMBER, f.em(5.6), stamped, f.rect.w - 2 * f.len(8.0));
    emboss(p, f, f.x(8.0), f.y(29.4), NUMBER, number_em, stamped);
    emboss(p, f, f.x(25.0), f.y(39.4), EXPIRY, f.em(3.6), stamped);
    emboss(p, f, f.x(8.0), f.y(46.2), CARDHOLDER, f.em(3.6), stamped);
}

/// Largest em at or below `em` that keeps `text` within `max_w` device pixels.
fn fit(p: &Painter, text: &str, em: f32, family: FontFamily, max_w: i32) -> f32 {
    let w = p.measure_text_styled(text, em, family, FontStyle::Bold).w;
    if w <= max_w || w <= 0 {
        em
    } else {
        em * max_w as f32 / w as f32
    }
}

fn emboss(p: &mut Painter, f: &Face, x: i32, y: i32, text: &str, em: f32, family: FontFamily) {
    let relief = f.len(0.3);
    p.text_styled(
        x + relief,
        y + relief,
        text,
        em,
        RAISED_SHADOW,
        family,
        FontStyle::Bold,
    );
    p.text_styled(x, y, text, em, RAISED, family, FontStyle::Bold);
}

/// The network mark: two overlapping discs, filled span by span so the lens
/// where they cross can take a third colour — the painter blends nothing, so an
/// overlap has to be drawn rather than composited.
fn paint_mark(p: &mut Painter, f: &Face) {
    let radius = f.len(4.8);
    let (left, right, centre) = (f.x(68.6), f.x(75.0), f.y(45.4));

    for dy in -radius..=radius {
        let half = (((radius * radius - dy * dy).max(0)) as f32).sqrt().round() as i32;
        if half <= 0 {
            continue;
        }
        let y = centre + dy;
        p.h_line(left - half, y, 2 * half + 1, DISC_LEFT);
        p.h_line(right - half, y, 2 * half + 1, DISC_RIGHT);
        let (x0, x1) = (right - half, left + half);
        if x1 >= x0 {
            p.h_line(x0, y, x1 - x0 + 1, DISC_OVERLAP);
        }
    }
}

// ---------------------------------------------------------------------------
// The numbers behind the drawing
// ---------------------------------------------------------------------------

/// The lines [`Fit::readout`] puts together, centred under the card.
struct Readout {
    rect: Rect,
    lines: [String; 3],
}

impl Widget for Readout {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let size = theme.font_size;
        let line_h = painter.measure_text("", size).h.max(1);
        let mut y = self.rect.y;
        for line in &self.lines {
            let row = Rect::new(self.rect.x, y, self.rect.w, line_h);
            painter.text_centered(row, line, size, theme.text);
            y += line_h;
        }
    }
}

/// A scale factor without its trailing zeros: 1.46 stays 1.46, 2.00 reads 2.
fn trim(scale: f32) -> String {
    let text = format!("{scale:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}
