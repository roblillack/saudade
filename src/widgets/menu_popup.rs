//! The drop-down panel shared by the two widgets that show one: a
//! [`MenuBar`](crate::widgets::MenuBar) menu hanging off a bar label, and a
//! [`ContextMenu`](crate::widgets::ContextMenu) anchored at the pointer.
//!
//! Everything about how a menu *looks* and how its rows are addressed lives
//! here — the white panel with its thin black border and L-shaped drop shadow,
//! the row metrics, the checkmark gutter, the right-aligned accelerator column,
//! separators, hit-testing and keyboard navigation. The two owners differ only
//! in where the panel is placed and what opens it, so both keep their own
//! geometry and hand it to [`MenuPopup`] per call.
//!
//! [`MenuPopup`] is a borrowed *view*, not a widget: it holds no state of its
//! own, so the owner stays the single place a menu's items, scroll offset and
//! highlight live.

use crate::accel::ModifierScheme;
use crate::geometry::{Color, Point, Rect, Size};
use crate::include_svg;
use crate::painter::Painter;
use crate::svg::SvgImage;
use crate::theme::Theme;
use crate::widgets::menu::MenuItem;
use crate::widgets::mnemonic::{draw_label_with_mnemonic, parse_label};

/// Horizontal inset from the panel edge to a row's label. Wide enough to leave
/// a gutter for the checkmark on the left.
pub(crate) const POPUP_PADDING_X: i32 = 18;
/// Breathing room above the first row and below the last — also the strip the
/// scroll arrows are drawn in when a panel is too short for its items.
pub(crate) const POPUP_PADDING_Y: i32 = 3;
pub(crate) const ITEM_HEIGHT: i32 = 18;
/// Gap between an item's label and its right-aligned accelerator hint.
const ACCEL_GAP: i32 = 24;
const ITEM_TEXT_INSET_Y: i32 = 1;
pub(crate) const SEPARATOR_HEIGHT: i32 = 6;
pub(crate) const SHADOW_SIZE: i32 = 2;
/// L-shape drop shadow color: a dark gray with no alpha trickery so it
/// renders crisply on every backend.
const SHADOW_COLOR: Color = Color::DARK_GRAY;
/// Footprint of the checkmark drawn in a checked item's left gutter. Kept a few
/// pixels smaller than the label inset (`POPUP_PADDING_X - 4`) and centered in
/// it, so the tick has breathing room on both sides and never crowds the text —
/// and so unchecked / uncheckable menus keep their exact layout.
const CHECK_SIZE: i32 = 9;
/// A tiny nudge applied to the centered checkmark so it reads as aligned with
/// the label rather than sitting a hair high and left of it.
const CHECK_NUDGE_X: i32 = 1;
const CHECK_NUDGE_Y: i32 = 1;
/// Height of a scroll arrow: three rows widening to a 5-px base.
const ARROW_H: i32 = 3;
/// Height claimed at a clipped end of a scrolled panel for its arrow. The
/// arrow is centered in it, so it sits clear of both the border and the first
/// (or last) row rather than crowding either.
const ARROW_STRIP: i32 = 7;

/// The checkmark glyph for a checked item, shared with the [`Checkbox`] widget
/// so both read identically. Baked black is a placeholder, tinted to the item's
/// text color via [`SvgImage::draw_tinted`].
///
/// [`Checkbox`]: crate::widgets::Checkbox
const CHECK: SvgImage = include_svg!("assets/checkbox/check.svg");

/// The footprint a popup *window* has to cover for a panel drawn at `rect`:
/// the panel plus the L-shaped drop shadow hanging off its right and bottom
/// edges, which would otherwise be clipped away at the window's edge.
pub(crate) fn with_shadow(rect: Rect) -> Rect {
    Rect::new(rect.x, rect.y, rect.w + SHADOW_SIZE, rect.h + SHADOW_SIZE)
}

/// How a panel's items land inside it: where the first drawn row starts, how
/// many are drawn, and which ends continue past the panel. Produced by
/// [`MenuPopup::rows`], which is the single place painting and hit-testing both
/// take their row layout from.
pub(crate) struct Rows {
    /// Offset from the panel's top edge to the first drawn row.
    pub top: i32,
    /// How many items are drawn, starting at the scroll offset.
    pub count: usize,
    /// Whether an arrow strip is claiming space at that end because items
    /// continue past it.
    pub up: bool,
    pub down: bool,
}

/// A borrowed view over one menu's items: everything the shared panel code
/// needs beyond the geometry its owner passes in.
pub(crate) struct MenuPopup<'a> {
    items: &'a [MenuItem],
    /// Scheme accelerator hints are rendered through — `Ctrl+R` on a PC,
    /// `⌘R` on a Mac.
    scheme: ModifierScheme,
}

impl<'a> MenuPopup<'a> {
    pub(crate) fn new(items: &'a [MenuItem], scheme: ModifierScheme) -> Self {
        Self { items, scheme }
    }

    /// The size the panel wants: as wide as the widest label — plus an
    /// accelerator column when any item carries a chord — and as tall as every
    /// item stacked, padding included.
    pub(crate) fn measure(&self, painter: &Painter, theme: &Theme) -> Size {
        let mut max_label = 0;
        let mut max_accel = 0;
        for item in self.items {
            if let MenuItem::Action { label, accel, .. } = item {
                let parsed = parse_label(label);
                let w = painter.measure_text(&parsed.display, theme.font_size).w;
                if w > max_label {
                    max_label = w;
                }
                if let Some(accel) = accel {
                    let aw = painter
                        .measure_text(&accel.label(self.scheme), theme.font_size)
                        .w;
                    if aw > max_accel {
                        max_accel = aw;
                    }
                }
            }
        }
        // The accelerator column only widens the popup when some item carries
        // one, so accelerator-free menus keep their original width.
        let accel_col = if max_accel > 0 {
            ACCEL_GAP + max_accel
        } else {
            0
        };
        let width = max_label + accel_col + POPUP_PADDING_X * 2;
        let height = POPUP_PADDING_Y * 2 + self.content_height();
        Size::new(width, height)
    }

    /// Total height of every item stacked, without the panel's padding.
    fn content_height(&self) -> i32 {
        self.items.iter().map(|item| item.height()).sum()
    }

    /// Which items a panel `height` tall shows at scroll offset `scroll`, and
    /// where they start. A panel measured at its natural height fits them all;
    /// a shorter one — a context menu capped to the window — shows a window of
    /// them, with an arrow strip at whichever end continues.
    pub(crate) fn rows(&self, height: i32, scroll: usize) -> Rows {
        let up = scroll > 0;
        let top = POPUP_PADDING_Y + if up { ARROW_STRIP } else { 0 };
        let mut avail = height - top - POPUP_PADDING_Y;
        let mut count = self.fit(avail, scroll);
        // Items left below need a strip of their own, which costs a row — so
        // the count has to be taken again against what is left. At least one
        // row is drawn regardless: a panel with nothing pickable in it is worse
        // than one whose single row is a little tight.
        if scroll + count < self.items.len() {
            avail -= ARROW_STRIP;
            count = self.fit(avail, scroll).max(1);
        }
        Rows {
            top,
            count,
            up,
            down: scroll + count < self.items.len(),
        }
    }

    /// How many items fit in `avail` pixels, starting at `scroll`.
    fn fit(&self, avail: i32, scroll: usize) -> usize {
        let mut used = 0;
        let mut count = 0;
        for item in self.items.iter().skip(scroll) {
            used += item.height();
            if used > avail {
                break;
            }
            count += 1;
        }
        count
    }

    /// The largest scroll offset worth having: the first one whose window
    /// reaches the last item, so scrolling can't run off the end into empty
    /// rows.
    pub(crate) fn max_scroll(&self, height: i32) -> usize {
        (0..self.items.len())
            .find(|&scroll| !self.rows(height, scroll).down)
            .unwrap_or(0)
    }

    /// The arrow strips of a panel that had to clip its items: the rect at the
    /// top when there is more above, the one at the bottom when there is more
    /// below. Clicking either scrolls that way.
    pub(crate) fn arrow_strips(&self, rect: Rect, scroll: usize) -> (Option<Rect>, Option<Rect>) {
        let rows = self.rows(rect.h, scroll);
        let strip = |y: i32| Rect::new(rect.x + 1, y, rect.w - 2, ARROW_STRIP);
        (
            rows.up.then(|| strip(rect.y + POPUP_PADDING_Y)),
            rows.down
                .then(|| strip(rect.bottom() - POPUP_PADDING_Y - ARROW_STRIP)),
        )
    }

    /// Index of the selectable item under `pos`, for a panel drawn at `rect`
    /// scrolled to `scroll`. Separators and disabled items answer `None`, so a
    /// press on one neither highlights nor fires anything.
    pub(crate) fn hit(&self, rect: Rect, scroll: usize, pos: Point) -> Option<usize> {
        if !rect.contains(pos) {
            return None;
        }
        let rows = self.rows(rect.h, scroll);
        let mut y = rect.y + rows.top;
        for (i, item) in self.items.iter().enumerate().skip(scroll).take(rows.count) {
            let h = item.height();
            if pos.y >= y && pos.y < y + h {
                return item.is_selectable().then_some(i);
            }
            y += h;
        }
        None
    }

    /// Draw the panel at `rect`: shadow, white interior, thin black border, and
    /// the window of items starting at `scroll` with `hovered` highlighted.
    pub(crate) fn paint(
        &self,
        painter: &mut Painter,
        theme: &Theme,
        rect: Rect,
        scroll: usize,
        hovered: Option<usize>,
    ) {
        // L-shape drop shadow drawn first so the popup overlays it on the
        // top/left edges.
        painter.fill_rect(
            Rect::new(rect.x + SHADOW_SIZE, rect.bottom(), rect.w, SHADOW_SIZE),
            SHADOW_COLOR,
        );
        painter.fill_rect(
            Rect::new(rect.right(), rect.y + SHADOW_SIZE, SHADOW_SIZE, rect.h),
            SHADOW_COLOR,
        );

        // White interior + thin black border. No raised bevel — Win 3.1
        // drop-downs are flat panels, the bar holds the chrome.
        painter.fill_rect(rect, theme.background);
        painter.stroke_rect(rect, theme.border);

        let rows = self.rows(rect.h, scroll);
        let mut y = rect.y + rows.top;
        for (i, item) in self.items.iter().enumerate().skip(scroll).take(rows.count) {
            match item {
                MenuItem::Action { label, accel, .. } => {
                    let row = Rect::new(rect.x + 1, y, rect.w - 2, ITEM_HEIGHT);
                    let parsed = parse_label(label);
                    // A disabled item is greyed and never shows the hover band
                    // (it can't be hovered — `hit` skips it).
                    let (bg, fg) = if !item.is_enabled() {
                        (theme.background, theme.disabled_text)
                    } else if hovered == Some(i) {
                        (theme.highlight_bg, theme.highlight_text)
                    } else {
                        (theme.background, theme.text)
                    };
                    painter.fill_rect(row, bg);
                    // A checked item gets a tick centered in the left gutter,
                    // tinted to match the (possibly greyed / highlighted) label
                    // color. It rides inside the existing label inset, so the
                    // text never shifts whether or not the item is checked. A
                    // 1px nudge down/right sits it more squarely against the
                    // label's optical baseline.
                    if item.is_checked() {
                        let gutter = POPUP_PADDING_X - 4;
                        let cx = row.x + (gutter - CHECK_SIZE) / 2 + CHECK_NUDGE_X;
                        let cy = row.y + (ITEM_HEIGHT - CHECK_SIZE) / 2 + CHECK_NUDGE_Y;
                        let check = Rect::new(cx, cy, CHECK_SIZE, CHECK_SIZE);
                        CHECK.draw_tinted(painter, check, fg);
                    }
                    draw_label_with_mnemonic(
                        painter,
                        row.x + POPUP_PADDING_X - 4,
                        row.y + ITEM_TEXT_INSET_Y,
                        0,
                        &parsed,
                        theme.font_size,
                        fg,
                    );
                    // Accelerator hint, right-aligned with the same inset the
                    // label carries on the left.
                    if let Some(accel) = accel {
                        let hint = accel.label(self.scheme);
                        let aw = painter.measure_text(&hint, theme.font_size).w;
                        let ax = row.right() - (POPUP_PADDING_X - 4) - aw;
                        painter.text(ax, row.y + ITEM_TEXT_INSET_Y, &hint, theme.font_size, fg);
                    }
                    y += ITEM_HEIGHT;
                }
                MenuItem::Separator => {
                    let mid = y + SEPARATOR_HEIGHT / 2;
                    painter.etched_h_line(rect.x + 4, mid, rect.w - 8, theme);
                    y += SEPARATOR_HEIGHT;
                }
            }
        }

        // A panel that had to clip its items says so, with an arrow centered in
        // the strip that end reserved for it.
        let (up, down) = self.arrow_strips(rect, scroll);
        for (strip, up) in [(up, true), (down, false)] {
            if let Some(strip) = strip {
                let y = strip.y + (ARROW_STRIP - ARROW_H) / 2;
                arrow(painter, rect, y, up, theme.text);
            }
        }
    }

    /// Index of the first selectable item (skipping separators and disabled
    /// rows); `None` if the menu has none.
    pub(crate) fn first_action(&self) -> Option<usize> {
        self.items.iter().position(|item| item.is_selectable())
    }

    pub(crate) fn last_action(&self) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, item)| item.is_selectable().then_some(i))
    }

    /// Step the highlight by ±1 selectable item, skipping separators and
    /// disabled rows and wrapping at both ends. `from` is the current
    /// highlight, `delta` is +1 (Down) or -1 (Up). `None` when the menu holds
    /// nothing selectable.
    pub(crate) fn step(&self, from: Option<usize>, delta: i32) -> Option<usize> {
        let actions: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_selectable())
            .map(|(i, _)| i)
            .collect();
        if actions.is_empty() {
            return None;
        }
        let current = from.and_then(|h| actions.iter().position(|&a| a == h));
        let next = match (current, delta) {
            (None, 1) => 0,
            (None, _) => actions.len() - 1,
            (Some(i), d) => {
                let len = actions.len() as i32;
                ((i as i32 + d).rem_euclid(len)) as usize
            }
        };
        Some(actions[next])
    }

    /// Index of the selectable item whose mnemonic is `ch`.
    pub(crate) fn mnemonic(&self, ch: char) -> Option<usize> {
        let target = ch.to_ascii_lowercase();
        self.items.iter().enumerate().find_map(|(i, item)| {
            let MenuItem::Action { label, .. } = item else {
                return None;
            };
            (item.is_enabled() && parse_label(label).mnemonic_char == Some(target)).then_some(i)
        })
    }
}

/// A three-line triangle centered in the panel: widening downwards for a down
/// arrow, upwards for an up arrow.
fn arrow(painter: &mut Painter, rect: Rect, y: i32, up: bool, color: Color) {
    let cx = rect.x + rect.w / 2;
    for step in 0..ARROW_H {
        let half = if up { step } else { ARROW_H - 1 - step };
        painter.h_line(cx - half, y + step, half * 2 + 1, color);
    }
}
