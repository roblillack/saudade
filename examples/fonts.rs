//! fonts — a specimen sheet of every face saudade can draw with.
//!
//! The runtime asks the host for three families — a proportional sans-serif, a
//! proportional serif and a monospace — and loads each one's *real* bold,
//! italic and bold-italic faces alongside its regular one, so twelve faces are
//! reachable through [`Painter::text_styled`]. This example puts all of them on
//! screen at once: one row per face, grouped into a band per family, one column
//! per size, every cell the same word drawn on the row's shared baseline so two
//! faces can be compared straight down a column.
//!
//! Sizes are logical pixels — what the `size` argument of `Painter::text` takes,
//! before the DPI scale is applied. The column matching [`Theme::font_size`],
//! which every piece of chrome in this window is drawn at, is highlighted.
//!
//! A style the host ships no real face for falls back to the nearest one it does
//! have, and saudade never synthesizes (smears / shears) a missing face — so two
//! identical-looking rows here mean that family really only has the one face.
//!
//! The table is bigger than a sensible window, so it scrolls both ways — wheel,
//! [`ScrollBar`]s, or the arrow / Page / Home / End keys — with the size header
//! and the style column frozen in place.

use saudade::{
    App, Color, Event, EventCtx, FontFamily, FontStyle, Key, NamedKey, Painter, Rect,
    SCROLLBAR_THICKNESS, ScrollBar, Theme, Widget, WindowConfig,
};

const W: i32 = 800;
const H: i32 = 610;

/// Drawn in every cell: an ascender, a descender, round and straight stems —
/// about the least alphabet that still tells two faces apart at 8 px.
const SAMPLE: &str = "Hamburg";

/// The table's columns, in logical pixels: the classic Win 3.1 size ladder, cut
/// off where a row stops fitting on a normal screen.
const SIZES: [f32; 9] = [8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 20.0, 24.0];

/// The row groups — one band per family the runtime loads.
const FAMILIES: [(FontFamily, &str); 3] = [
    (FontFamily::Sans, "Sans-serif"),
    (FontFamily::Serif, "Serif"),
    (FontFamily::Mono, "Monospace"),
];

/// The rows inside a band — one per face of that family.
const STYLES: [(FontStyle, &str); 4] = [
    (FontStyle::Regular, "Regular"),
    (FontStyle::Bold, "Bold"),
    (FontStyle::Italic, "Italic"),
    (FontStyle::BoldItalic, "Bold Italic"),
];

/// Gap between the window edge and the table.
const MARGIN: i32 = 8;
/// Room for the footnote under the table.
const FOOTER_H: i32 = 18;
/// Breathing room left and right of a cell's contents.
const CELL_PAD: i32 = 6;
/// Space above and below the sample in a face row.
const ROW_PAD: i32 = 5;
/// How far a style row is indented under its family band.
const INDENT: i32 = 10;
/// What one press of Left / Right scrolls.
const H_STEP: i32 = 24;
/// The hairline ruling between cells — lighter than `theme.shadow`, which would
/// read as chrome rather than as ruling.
const GRID_LINE: Color = Color::rgb(0xD0, 0xD0, 0xD0);

fn main() {
    App::new(
        WindowConfig::new("Fonts", W, H)
            .resizable(true)
            .min_size(320, 200),
        Specimen::new(),
    )
    .run();
}

/// One line of the table.
#[derive(Clone, Copy)]
enum Row {
    /// A family band: a full-width caption above that family's four faces.
    Band(usize),
    /// One face, as `(family index, style index)`.
    Face(usize, usize),
}

/// The table's lines, top to bottom.
fn rows() -> Vec<Row> {
    let mut out = Vec::with_capacity(FAMILIES.len() * (STYLES.len() + 1));
    for f in 0..FAMILIES.len() {
        out.push(Row::Band(f));
        out.extend((0..STYLES.len()).map(|s| Row::Face(f, s)));
    }
    out
}

/// A column's caption: the size, without a trailing `.0` on the whole ones.
fn size_label(size: f32) -> String {
    if size.fract() == 0.0 {
        format!("{}", size as i32)
    } else {
        format!("{size}")
    }
}

/// Where a line of `size` text starts so that it comes out vertically centered
/// in `rect` — `Painter::text` places the em box's top at the y it is given.
fn centered_text_y(rect: Rect, size: f32) -> i32 {
    rect.y + (rect.h - (size * 1.2).ceil() as i32) / 2
}

/// The table's measurements.
///
/// Column widths depend on the host's *actual* faces — a monospace "Hamburg" is
/// half again as wide as a sans one — so this is measured rather than baked into
/// constants, and re-measured on every paint: each measurement is a walk over
/// the font's per-glyph advance cache, which the same frame's drawing fills
/// anyway.
struct Metrics {
    /// Width of the frozen style column.
    label_w: i32,
    /// Height of the frozen size header.
    head_h: i32,
    /// Height of a family band.
    band_h: i32,
    /// Height of a face row: the largest sample plus its descender.
    row_h: i32,
    /// Where a face row's shared baseline sits below the row's top edge.
    baseline: i32,
    /// Width of each size column, in `SIZES` order.
    cols: Vec<i32>,
    /// The scrolling part's size — the cells alone, without the frozen header
    /// row and style column.
    content_w: i32,
    content_h: i32,
}

impl Metrics {
    fn measure(painter: &Painter, theme: &Theme) -> Self {
        let ui = theme.font_size;
        let max_size = SIZES.iter().copied().fold(0.0_f32, f32::max);

        // The style column fits its widest caption, indented under the band.
        let mut label_w = painter.measure_text("Style", ui).w;
        for (_, name) in STYLES {
            label_w = label_w.max(INDENT + painter.measure_text(name, ui).w);
        }

        // A column is as wide as the widest face draws the sample at that size,
        // and never narrower than its own header.
        let mut cols = Vec::with_capacity(SIZES.len());
        for &size in &SIZES {
            let mut w = painter.measure_text(&size_label(size), ui).w;
            for (family, _) in FAMILIES {
                for (style, _) in STYLES {
                    w = w.max(painter.measure_text_styled(SAMPLE, size, family, style).w);
                }
            }
            cols.push(w + 2 * CELL_PAD);
        }

        let head_h = (ui * 1.2).ceil() as i32 + 8;
        let row_h = (max_size * 1.2).ceil() as i32 + 2 * ROW_PAD;

        Self {
            label_w: label_w + 2 * CELL_PAD,
            head_h,
            band_h: head_h,
            row_h,
            // Every cell in a row shares the largest size's baseline, so the
            // samples sit on one line and grow upward across the row.
            baseline: ROW_PAD + max_size.ceil() as i32,
            content_w: cols.iter().sum(),
            content_h: FAMILIES.len() as i32 * (head_h + STYLES.len() as i32 * row_h),
            cols,
        }
    }

    /// The height of one line of the table.
    fn height(&self, row: Row) -> i32 {
        match row {
            Row::Band(_) => self.band_h,
            Row::Face(..) => self.row_h,
        }
    }
}

/// The whole window: the table, its two scrollbars, and the footnote.
struct Specimen {
    bounds: Rect,
    vbar: ScrollBar,
    hbar: ScrollBar,
    /// Last painted face-row height — what one press of Up / Down scrolls.
    /// Measured, so the keys move by exactly one row whatever faces the host
    /// turns out to have.
    row_h: i32,
}

impl Specimen {
    fn new() -> Self {
        // The bars are placed and ranged in `paint`, once the window size and
        // the content they have to carry are both known.
        Self {
            bounds: Rect::new(0, 0, W, H),
            vbar: ScrollBar::vertical(Rect::new(0, 0, 0, 0)),
            hbar: ScrollBar::horizontal(Rect::new(0, 0, 0, 0)),
            row_h: 1,
        }
    }

    /// The bordered table, filling the window above the footnote.
    fn table(&self) -> Rect {
        let b = self.bounds;
        Rect::new(
            b.x + MARGIN,
            b.y + MARGIN,
            (b.w - 2 * MARGIN).max(0),
            (b.h - 2 * MARGIN - FOOTER_H).max(0),
        )
    }

    /// The table's interior: inside the border, minus the two scrollbar gutters.
    fn grid(&self) -> Rect {
        let t = self.table();
        Rect::new(
            t.x + 1,
            t.y + 1,
            (t.w - SCROLLBAR_THICKNESS - 1).max(0),
            (t.h - SCROLLBAR_THICKNESS - 1).max(0),
        )
    }

    fn vbar_rect(&self) -> Rect {
        let t = self.table();
        Rect::new(
            t.right() - SCROLLBAR_THICKNESS,
            t.y,
            SCROLLBAR_THICKNESS,
            (t.h - SCROLLBAR_THICKNESS).max(0),
        )
    }

    fn hbar_rect(&self) -> Rect {
        let t = self.table();
        Rect::new(
            t.x,
            t.bottom() - SCROLLBAR_THICKNESS,
            (t.w - SCROLLBAR_THICKNESS).max(0),
            SCROLLBAR_THICKNESS,
        )
    }
}

impl Widget for Specimen {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn layout(&mut self, bounds: Rect) {
        // Track the live window size: everything below is derived from it.
        self.bounds = bounds;
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let m = Metrics::measure(painter, theme);
        let ui = theme.font_size;
        let table = self.table();
        let grid = self.grid();
        self.row_h = m.row_h;

        // The frozen header and column split the interior into four panes: a
        // dead corner, the sizes across the top (scrolling sideways only), the
        // faces down the left (vertically only), and the cells, the one pane
        // that moves both ways.
        let corner = Rect::new(grid.x, grid.y, m.label_w.min(grid.w), m.head_h.min(grid.h));
        let head = Rect::new(
            grid.x + corner.w,
            grid.y,
            (grid.w - corner.w).max(0),
            corner.h,
        );
        let side = Rect::new(
            grid.x,
            grid.y + corner.h,
            corner.w,
            (grid.h - corner.h).max(0),
        );
        let body = Rect::new(head.x, side.y, head.w, side.h);

        self.vbar.set_rect(self.vbar_rect());
        self.hbar.set_rect(self.hbar_rect());
        // Both bars count pixels of content, so a line is a row's worth of them
        // and a page is the visible pane.
        self.vbar.set_range(body.h, (m.content_h - body.h).max(0));
        self.hbar.set_range(body.w, (m.content_w - body.w).max(0));
        self.vbar.set_line_step(m.row_h);
        self.hbar.set_line_step(H_STEP);
        let (sx, sy) = (self.hbar.value(), self.vbar.value());

        painter.fill_rect(table, theme.face);

        // The cells, on the white ground of a list field. Rows scrolled out of
        // sight are skipped rather than clipped away glyph by glyph.
        let saved = painter.push_clip(body);
        painter.fill_rect(body, Color::WHITE);
        let mut y = body.y - sy;
        for row in rows() {
            if let Row::Face(f, s) = row
                && y < body.bottom()
                && y + m.row_h > body.y
            {
                let rect = Rect::new(body.x - sx, y, m.content_w, m.row_h);
                paint_face_row(painter, theme, &m, rect, FAMILIES[f].0, STYLES[s].0);
                painter.h_line(body.x, rect.bottom() - 1, body.w, GRID_LINE);
            }
            y += m.height(row);
        }
        painter.restore_clip(saved);

        // The frozen style column.
        let saved = painter.push_clip(side);
        painter.fill_rect(side, theme.face);
        let mut y = side.y - sy;
        for row in rows() {
            if let Row::Face(_, s) = row {
                let cell = Rect::new(side.x, y, side.w, m.row_h);
                paint_header(painter, theme, cell, false);
                painter.text(
                    cell.x + CELL_PAD + INDENT,
                    centered_text_y(cell, ui),
                    STYLES[s].1,
                    ui,
                    theme.text,
                );
            }
            y += m.height(row);
        }
        painter.restore_clip(saved);

        // The family bands run across both of those panes, so they are painted
        // over the top of them, clipped to the pair.
        let bands = Rect::new(grid.x, body.y, grid.w, body.h);
        let saved = painter.push_clip(bands);
        let mut y = bands.y - sy;
        for row in rows() {
            if let Row::Band(f) = row {
                let (family, name) = FAMILIES[f];
                let rect = Rect::new(bands.x, y, bands.w, m.band_h);
                paint_band(painter, theme, rect, name, has_family(painter, family));
            }
            y += m.height(row);
        }
        painter.restore_clip(saved);

        // The frozen size header. The column the chrome itself is drawn at gets
        // the selected look.
        let saved = painter.push_clip(head);
        painter.fill_rect(head, theme.face);
        let mut x = head.x - sx;
        for (i, &size) in SIZES.iter().enumerate() {
            let cell = Rect::new(x, head.y, m.cols[i], head.h);
            let is_ui = (size - ui).abs() < 0.01;
            paint_header(painter, theme, cell, is_ui);
            let fg = if is_ui {
                theme.highlight_text
            } else {
                theme.text
            };
            painter.text_centered(cell, &size_label(size), ui, fg);
            x += m.cols[i];
        }
        painter.restore_clip(saved);

        paint_header(painter, theme, corner, false);
        painter.text_centered(corner, "Style", ui, theme.text);

        // The dead square where the two bars meet, then the bars, then the
        // outline over the lot of them.
        painter.fill_rect(
            Rect::new(
                table.right() - SCROLLBAR_THICKNESS,
                table.bottom() - SCROLLBAR_THICKNESS,
                SCROLLBAR_THICKNESS,
                SCROLLBAR_THICKNESS,
            ),
            theme.face,
        );
        self.vbar.paint(painter, theme);
        self.hbar.paint(painter, theme);
        painter.stroke_rect(table, theme.border);

        painter.text(
            table.x,
            table.bottom() + 4,
            "Sizes in logical pixels. Highlighted: the theme's chrome size. \
             A missing face falls back to the nearest real one.",
            ui,
            theme.text,
        );
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // A held arrow button or a thumb drag owns every event until it lets go
        // — including the ticks that drive the auto-repeat.
        if self.vbar.captures_pointer() {
            self.vbar.event(event, ctx);
            return;
        }
        if self.hbar.captures_pointer() {
            self.hbar.event(event, ctx);
            return;
        }

        match event {
            // The wheel scrolls wherever it is over the table; a horizontal
            // wheel or a trackpad's sideways gesture drives the other bar (each
            // bar reads only its own axis out of the event).
            Event::Scroll { pos, .. } if self.table().contains(*pos) => {
                self.vbar.event(event, ctx);
                self.hbar.event(event, ctx);
            }
            Event::KeyDown {
                key: Key::Named(named),
                ..
            } => {
                let (line, page) = (self.row_h, self.vbar.viewport().max(1));
                let before = (self.vbar.value(), self.hbar.value());
                match named {
                    NamedKey::Up => self.vbar.set_value(before.0 - line),
                    NamedKey::Down => self.vbar.set_value(before.0 + line),
                    NamedKey::PageUp => self.vbar.set_value(before.0 - page),
                    NamedKey::PageDown => self.vbar.set_value(before.0 + page),
                    NamedKey::Home => self.vbar.set_value(0),
                    NamedKey::End => self.vbar.set_value(self.vbar.max()),
                    NamedKey::Left => self.hbar.set_value(before.1 - H_STEP),
                    NamedKey::Right => self.hbar.set_value(before.1 + H_STEP),
                    _ => return,
                }
                if (self.vbar.value(), self.hbar.value()) != before {
                    ctx.request_paint();
                }
            }
            _ => {
                if let Some(pos) = event.position() {
                    if self.vbar.rect().contains(pos) {
                        self.vbar.event(event, ctx);
                    } else if self.hbar.rect().contains(pos) {
                        self.hbar.event(event, ctx);
                    }
                }
            }
        }
    }

    fn captures_pointer(&self) -> bool {
        self.vbar.captures_pointer() || self.hbar.captures_pointer()
    }
}

/// Whether the host gave us a font for `family` at all. A family it has none
/// for draws nothing, so its band says so rather than leaving four blank rows.
fn has_family(painter: &Painter, family: FontFamily) -> bool {
    match family {
        FontFamily::Sans => painter.font().is_some(),
        FontFamily::Serif => painter.serif_font().is_some(),
        FontFamily::Mono => painter.mono_font().is_some(),
    }
}

/// One face's cells, left to right on the row's shared baseline. `row` spans the
/// full content width and is already offset by the horizontal scroll.
fn paint_face_row(
    painter: &mut Painter,
    theme: &Theme,
    m: &Metrics,
    row: Rect,
    family: FontFamily,
    style: FontStyle,
) {
    let mut x = row.x;
    for (i, &size) in SIZES.iter().enumerate() {
        let w = m.cols[i];
        painter.v_line(x + w - 1, row.y, row.h, GRID_LINE);
        painter.text_styled(
            x + CELL_PAD,
            row.y + m.baseline - size.round() as i32,
            SAMPLE,
            size,
            theme.text,
            family,
            style,
        );
        x += w;
    }
}

/// A family band: the name in bold, ruled off above and below.
fn paint_band(painter: &mut Painter, theme: &Theme, rect: Rect, name: &str, available: bool) {
    let ui = theme.font_size;
    painter.fill_rect(rect, theme.face);
    painter.h_line(rect.x, rect.y, rect.w, theme.shadow);
    painter.h_line(rect.x, rect.bottom() - 1, rect.w, theme.shadow);

    let y = centered_text_y(rect, ui);
    let x = rect.x + CELL_PAD;
    painter.text_styled(
        x,
        y,
        name,
        ui,
        theme.text,
        FontFamily::Sans,
        FontStyle::Bold,
    );
    if !available {
        let w = painter
            .measure_text_styled(name, ui, FontFamily::Sans, FontStyle::Bold)
            .w;
        painter.text(
            x + w + CELL_PAD,
            y,
            "- no font for this family on this system",
            ui,
            theme.disabled_text,
        );
    }
}

/// A raised header cell, inverted when it is the one being pointed out. The
/// caller draws the label, since the header row centers its own and the style
/// column left-aligns its own.
fn paint_header(painter: &mut Painter, theme: &Theme, rect: Rect, selected: bool) {
    painter.button(rect, theme, false, false);
    if selected {
        painter.fill_rect(rect.inset(1), theme.highlight_bg);
    }
}

// ============================================================================
// Tests — the table measures itself against whatever faces the host turns out
// to have, so what is worth checking is not the numbers but that it scrolls to
// every part of itself in a window too small to hold it.
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use saudade::mock::MockBackend;
    use saudade::{Font, Modifiers, Point};

    /// A window deliberately smaller than the table, so there is something to
    /// scroll on both axes whatever this machine's fonts measure.
    const SMALL_W: i32 = 400;
    const SMALL_H: i32 = 300;

    /// A backend carrying the faces this machine actually has — the same ones
    /// the running example draws, and what the table sizes its columns to.
    fn small() -> MockBackend {
        let mut backend = MockBackend::new(SMALL_W, SMALL_H);
        if let Some(font) = Font::load_sans() {
            backend = backend.with_sans_font(font);
        }
        if let Some(font) = Font::load_serif() {
            backend = backend.with_serif_font(font);
        }
        if let Some(font) = Font::load_monospace() {
            backend = backend.with_mono_font(font);
        }
        backend
    }

    fn press(backend: &MockBackend, ui: &mut Specimen, key: NamedKey) {
        backend.dispatch(
            ui,
            &Event::KeyDown {
                key: Key::Named(key),
                modifiers: Modifiers::default(),
            },
        );
    }

    #[test]
    fn a_window_too_short_for_the_faces_scrolls_to_the_last_row() {
        let backend = small();
        let mut ui = Specimen::new();
        backend.render(&mut ui);

        // Twelve faces at 24 px never fit 300 px of window, whatever the host's
        // fonts measure: a row's height comes from `SIZES`, not from them.
        assert!(ui.vbar.max() > 0, "the faces should outgrow the window");

        press(&backend, &mut ui, NamedKey::End);
        assert_eq!(ui.vbar.value(), ui.vbar.max());
        // Paints the tail — where every row above the viewport is skipped and
        // the last one is the one clipped in half.
        backend.render(&mut ui);

        press(&backend, &mut ui, NamedKey::Home);
        assert_eq!(ui.vbar.value(), 0);
    }

    #[test]
    fn the_wheel_scrolls_the_cells_wherever_it_is_over_the_table() {
        let backend = small();
        let mut ui = Specimen::new();
        backend.render(&mut ui);

        let wheel = |dy: f32| Event::Scroll {
            pos: Point::new(SMALL_W / 2, SMALL_H / 2),
            delta_x: 0.0,
            delta_y: dy,
        };
        backend.dispatch(&mut ui, &wheel(3.0));
        assert!(ui.vbar.value() > 0, "a detent down should move the cells");

        // And back up: the bar stops at the top rather than running negative.
        backend.dispatch(&mut ui, &wheel(-9.0));
        assert_eq!(ui.vbar.value(), 0);
    }

    #[test]
    fn a_window_too_narrow_for_the_columns_scrolls_sideways() {
        let backend = small();
        let mut ui = Specimen::new();
        backend.render(&mut ui);

        // Nothing to measure without fonts: with no faces to draw, the samples
        // measure zero and the columns shrink to fit their own headers.
        if Font::load_sans().is_none() {
            return;
        }
        assert!(
            ui.hbar.max() > 0,
            "nine columns of samples can't fit 400 px"
        );

        press(&backend, &mut ui, NamedKey::Right);
        assert_eq!(ui.hbar.value(), H_STEP);
        // Repaints with the cells offset under a header that stayed put.
        backend.render(&mut ui);
    }
}
