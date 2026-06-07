//! Window chrome for framed screenshots.
//!
//! Saudade itself never draws its own title bar or frame — on a real desktop
//! that is the window manager's job. For automated screenshots, though, it is
//! often nice to capture a window the way a user actually sees it: sitting on
//! the desktop, inside a title bar and frame, with a drop shadow. This module
//! draws that chrome around a rendered widget tree.
//!
//! The look reproduces the default rendering style of **Canoe** (the Win 3.1
//! styled stacking window manager saudade is usually paired with): a teal
//! desktop, a soft drop shadow, a navy active title bar, light-gray beveled
//! window controls, and a black multi-layer border. Windows are always drawn
//! in their *active* (focused) state.
//!
//! The caller picks one of three frame styles via [`WindowFrame`], mirroring
//! the three Canoe paints for a window:
//!
//! * [`WindowFrame::Resizable`] — minimize + maximize buttons and the full
//!   multi-layer resize border.
//! * [`WindowFrame::Fixed`] — a minimize button only, and a single 1px outline
//!   instead of the chunky resize border.
//! * [`WindowFrame::Dialog`] — no minimize/maximize buttons, and the border's
//!   bulk layer takes the title-bar color so the frame reads as one with the
//!   title.
//!
//! Drive it through [`MockBackend::render_framed`](crate::mock::MockBackend::render_framed).

use crate::geometry::{Color, Rect, Size};
use crate::include_svg;
use crate::painter::Painter;
use crate::svg::SvgImage;

// --- Canoe default palette (its `UiConfig::default`), in saudade colors. ---

/// The teal Canoe desktop.
const DESKTOP_BACKGROUND: Color = Color::rgb(0x00, 0x80, 0x80);
/// Active title-bar fill (Win 3.1 navy).
const TITLEBAR_BG: Color = Color::NAVY;
/// Active title-bar text.
const TITLEBAR_TEXT: Color = Color::WHITE;
/// Outer/inner thin border layers, the separator, and the button dividers.
const BORDER_DARK: Color = Color::BLACK;
/// The bulk middle border layer for resizable / fixed windows.
const BORDER_MID: Color = Color::LIGHT_GRAY;
/// Window-control button face and its 3D bevel.
const BUTTON_BG: Color = Color::LIGHT_GRAY;
const BUTTON_HIGHLIGHT: Color = Color::WHITE;
const BUTTON_SHADOW: Color = Color::MID_GRAY;

// Title-bar control glyphs, baked from the same SVGs Canoe ships: a beveled
// horizontal bar for the close box, and filled triangles for minimize /
// maximize. The full 16-unit viewBox is preserved (no `crop`), so the marks
// keep the centered padding Canoe rasterizes them with.
const CLOSE_ICON: SvgImage = include_svg!("assets/chrome/close.svg");
const MINIMIZE_ICON: SvgImage = include_svg!("assets/chrome/minimize.svg");
const MAXIMIZE_ICON: SvgImage = include_svg!("assets/chrome/maximize.svg");

/// Soft shadow tint: black at ~20% (`0x00000033`). Drawn as pure black whose
/// alpha is `SHADOW_ALPHA` scaled by the falloff curve.
const SHADOW_ALPHA: f32 = 0x33 as f32;

// --- Canoe default metrics, in logical pixels (its `UiConfig::default`). ---

/// Point size of the title-bar text (and the basis for the bar's height).
const CHROME_FONT_SIZE: f32 = 12.0;
/// Resize-border thickness for resizable / dialog windows.
const BORDER_NORMAL: i32 = 4;
/// The 1px outline a fixed-size window collapses its border to.
const BORDER_FIXED: i32 = 1;
/// Active-window soft-shadow extent.
const SHADOW_SIZE: i32 = 20;
/// Default teal margin between the window (with its shadow) and the image edge.
const DEFAULT_MARGIN: i32 = 40;
/// Padding on each side of the title text within its slot.
const TITLE_PADDING: i32 = 6;

/// Title-bar height in logical pixels — Canoe's `0.75·font_size` rounded, then
/// doubled plus one so it is always odd (a clean center row).
fn titlebar_height() -> i32 {
    let base = (0.75 * CHROME_FONT_SIZE).round() as i32;
    (base * 2 + 1).max(1)
}

/// Square title-bar button / glyph cell size in logical pixels, derived from
/// the bar height the same way Canoe sizes its control icons.
fn icon_size() -> i32 {
    let tbh = titlebar_height();
    (tbh - 4).clamp(6, tbh.max(1))
}

/// Which Canoe frame style to draw around the captured window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowFrame {
    /// A standard resizable top-level window: minimize **and** maximize
    /// buttons, plus the full multi-layer resize border.
    Resizable,
    /// A non-resizable window: a minimize button only (no maximize), and the
    /// border collapsed to a single 1px outline (no resize border).
    Fixed,
    /// A dialog / child window: no minimize or maximize buttons, and the
    /// border's bulk layer takes the title-bar color so the frame reads as a
    /// continuation of the title.
    Dialog,
}

impl WindowFrame {
    /// Resize-border thickness this style reserves (logical px).
    fn border_width(self) -> i32 {
        match self {
            WindowFrame::Fixed => BORDER_FIXED,
            _ => BORDER_NORMAL,
        }
    }

    fn shows_minimize(self) -> bool {
        matches!(self, WindowFrame::Resizable | WindowFrame::Fixed)
    }

    fn shows_maximize(self) -> bool {
        matches!(self, WindowFrame::Resizable)
    }

    /// The bulk middle border layer color. Dialogs blend it into the title bar.
    fn mid_color(self) -> Color {
        match self {
            WindowFrame::Dialog => TITLEBAR_BG,
            _ => BORDER_MID,
        }
    }
}

/// How to wrap a screenshot in window chrome. Construct one of these and hand it
/// to [`MockBackend::render_framed`](crate::mock::MockBackend::render_framed).
///
/// The defaults reproduce Canoe's default desktop rendering; the builder
/// methods exist for the occasional screenshot that wants a different desktop
/// color or more / less breathing room.
///
/// ```no_run
/// use saudade::*;
/// use saudade::mock::MockBackend;
///
/// let mut root = Container::new(220, 64)
///     .with_background(Color::WHITE)
///     .add(Label::new(Rect::new(16, 24, 200, 16), "Ready."));
///
/// let snap = MockBackend::new(220, 64)
///     .render_framed(&mut root, &WindowChrome::resizable("My App"));
/// std::fs::write("shot.png", snap.to_png()).unwrap();
/// ```
#[derive(Clone, Debug)]
pub struct WindowChrome {
    title: String,
    frame: WindowFrame,
    desktop_background: Color,
    margin: i32,
}

impl WindowChrome {
    /// Chrome with the given window `title` and frame style, on the default
    /// teal desktop with the default margin.
    pub fn new(title: impl Into<String>, frame: WindowFrame) -> Self {
        Self {
            title: title.into(),
            frame,
            desktop_background: DESKTOP_BACKGROUND,
            margin: DEFAULT_MARGIN,
        }
    }

    /// A resizable window (minimize + maximize buttons, full resize border).
    pub fn resizable(title: impl Into<String>) -> Self {
        Self::new(title, WindowFrame::Resizable)
    }

    /// A fixed-size window (minimize button only, 1px outline border).
    pub fn fixed(title: impl Into<String>) -> Self {
        Self::new(title, WindowFrame::Fixed)
    }

    /// A dialog window (no minimize / maximize buttons, dialog-style frame).
    pub fn dialog(title: impl Into<String>) -> Self {
        Self::new(title, WindowFrame::Dialog)
    }

    /// Override the teal desktop backdrop.
    pub fn with_desktop_background(mut self, color: Color) -> Self {
        self.desktop_background = color;
        self
    }

    /// Override the margin (logical px) of desktop around the window. The drop
    /// shadow is drawn within this margin, so very small values clip it.
    pub fn with_margin(mut self, margin: i32) -> Self {
        self.margin = margin.max(0);
        self
    }
}

/// Physical-pixel geometry of a framed render, derived from the content size,
/// scale, and chrome. All rectangles are in the framed buffer's coordinate
/// space (origin top-left).
pub(crate) struct Metrics {
    /// Physical scale the content was rendered at; chrome text / glyphs match it.
    scale: f32,
    /// Full framed buffer size.
    pub buffer: Size,
    /// The window frame (border + title bar + content), shadow excluded.
    frame: Rect,
    /// The title-bar strip (inside the border, above the separator).
    titlebar: Rect,
    /// Where the content blits in: top-left and (== the rendered content) size.
    pub content: Rect,
    /// Resize-border thickness.
    bw: i32,
    /// Separator / thin-line / divider thickness (one logical px).
    unit: i32,
    /// Soft-shadow extent.
    shadow: i32,
}

/// Compute the framed geometry for a `content_phys`-sized (physical px) render
/// at `scale`. Chrome metrics are taken in logical px and scaled here the same
/// way the painter scales everything else, so the frame lines up with the
/// content at any DPI.
pub(crate) fn metrics(content_phys: Size, scale: f32, chrome: &WindowChrome) -> Metrics {
    let scale = scale.max(0.01);
    let px = |logical: i32| (logical as f32 * scale).round() as i32;

    let bw = px(chrome.frame.border_width());
    let tbh = px(titlebar_height());
    let unit = px(1).max(1);
    let margin = px(chrome.margin);
    let shadow = px(SHADOW_SIZE);

    let cw = content_phys.w.max(1);
    let ch = content_phys.h.max(1);

    // Border + title bar + 1px separator + content + border.
    let frame_w = cw + bw * 2;
    let frame_h = ch + tbh + unit + bw * 2;

    let buffer = Size::new(frame_w + margin * 2, frame_h + margin * 2);
    let frame = Rect::new(margin, margin, frame_w, frame_h);
    let titlebar = Rect::new(frame.x + bw, frame.y + bw, cw, tbh);
    let content = Rect::new(frame.x + bw, frame.y + bw + tbh + unit, cw, ch);

    Metrics {
        scale,
        buffer,
        frame,
        titlebar,
        content,
        bw,
        unit,
        shadow,
    }
}

/// Paint the desktop, shadow, border, title bar, buttons, and title into the
/// painter (which must cover [`Metrics::buffer`] at scale 1.0). The content
/// area is left untouched for the caller to blit the rendered widget tree into.
pub(crate) fn paint(painter: &mut Painter, m: &Metrics, chrome: &WindowChrome) {
    // Desktop backdrop, then the soft drop shadow over it.
    painter.fill(chrome.desktop_background);
    draw_shadow(painter, m);

    // Border ring(s). The interior is filled here and then mostly overdrawn by
    // the title bar / separator / content, leaving just the border layers.
    painter.fill_rect(m.frame, BORDER_DARK);
    if chrome.frame != WindowFrame::Fixed {
        let outer = m.unit;
        let inner = m.unit;
        let mid = (m.bw - outer - inner).max(0);
        painter.fill_rect(m.frame.inset(outer), chrome.frame.mid_color());
        painter.fill_rect(m.frame.inset(outer + mid), BORDER_DARK);
    }

    // Title bar, then the black separator line beneath it.
    painter.fill_rect(m.titlebar, TITLEBAR_BG);
    painter.fill_rect(
        Rect::new(m.titlebar.x, m.titlebar.bottom(), m.titlebar.w, m.unit),
        BORDER_DARK,
    );

    draw_buttons_and_title(painter, m, chrome);
}

/// The three title-bar button slots, laid out left-to-right then right-to-left
/// exactly as Canoe positions them: a close box at the far left, and the
/// maximize / minimize buttons hugging the right edge.
struct ButtonSlots {
    close: Rect,
    minimize: Option<Rect>,
    maximize: Option<Rect>,
}

fn button_slots(m: &Metrics, frame: WindowFrame) -> ButtonSlots {
    let size = m.titlebar.h; // square buttons, one bar tall
    let y = m.titlebar.y;
    let close = Rect::new(m.titlebar.x, y, size, size);

    let gap = m.unit.max(1);
    let right_edge = m.titlebar.right();
    let mut next_right = right_edge - size;

    let maximize = frame.shows_maximize().then(|| {
        let r = Rect::new(next_right, y, size, size);
        next_right -= size + gap;
        r
    });
    let minimize = frame
        .shows_minimize()
        .then(|| Rect::new(next_right, y, size, size));

    ButtonSlots {
        close,
        minimize,
        maximize,
    }
}

fn draw_buttons_and_title(painter: &mut Painter, m: &Metrics, chrome: &WindowChrome) {
    let slots = button_slots(m, chrome.frame);
    let unit = m.unit.max(1);
    let icon = (icon_size() as f32 * m.scale).round() as i32;

    // Close box: a flat gray face with a black divider on its right, holding the
    // beveled Win 3.1 control-menu bar.
    painter.fill_rect(slots.close, BUTTON_BG);
    draw_divider(painter, slots.close.right(), m, unit);
    CLOSE_ICON.draw(painter, icon_box(slots.close, icon));

    if let Some(min) = slots.minimize {
        draw_divider(painter, min.x - unit, m, unit);
        draw_raised_button(painter, min, unit);
        MINIMIZE_ICON.draw(painter, icon_box(min, icon));
    }
    if let Some(max) = slots.maximize {
        draw_divider(painter, max.x - unit, m, unit);
        draw_raised_button(painter, max, unit);
        MAXIMIZE_ICON.draw(painter, icon_box(max, icon));
    }

    draw_title(painter, m, &slots, chrome);
}

/// The centered `size`×`size` cell for a glyph inside a button.
fn icon_box(button: Rect, size: i32) -> Rect {
    Rect::new(
        button.x + (button.w - size) / 2,
        button.y + (button.h - size) / 2,
        size,
        size,
    )
}

/// A vertical black divider line `unit` wide spanning the title bar height.
fn draw_divider(painter: &mut Painter, x: i32, m: &Metrics, unit: i32) {
    if x < m.titlebar.x {
        return;
    }
    painter.fill_rect(Rect::new(x, m.titlebar.y, unit, m.titlebar.h), BORDER_DARK);
}

/// A raised 3D button: gray face, white highlight on the top/left, a doubled
/// gray shadow on the bottom/right — the bevel Canoe gives its min/max buttons.
fn draw_raised_button(painter: &mut Painter, r: Rect, unit: i32) {
    painter.fill_rect(r, BUTTON_BG);
    // Highlight: top and left edges.
    painter.fill_rect(Rect::new(r.x, r.y, r.w, unit), BUTTON_HIGHLIGHT);
    painter.fill_rect(Rect::new(r.x, r.y, unit, r.h), BUTTON_HIGHLIGHT);
    // Inner shadow.
    if r.w >= 3 * unit && r.h >= 3 * unit {
        painter.fill_rect(
            Rect::new(r.x + unit, r.bottom() - 2 * unit, r.w - 2 * unit, unit),
            BUTTON_SHADOW,
        );
        painter.fill_rect(
            Rect::new(r.right() - 2 * unit, r.y + unit, unit, r.h - 2 * unit),
            BUTTON_SHADOW,
        );
    }
    // Outer shadow: bottom and right edges.
    painter.fill_rect(Rect::new(r.x, r.bottom() - unit, r.w, unit), BUTTON_SHADOW);
    painter.fill_rect(Rect::new(r.right() - unit, r.y, unit, r.h), BUTTON_SHADOW);
}

/// Draw the title text: left-aligned after the close box, vertically centered in
/// the bar, and clipped to the space before the right-hand buttons.
fn draw_title(painter: &mut Painter, m: &Metrics, slots: &ButtonSlots, chrome: &WindowChrome) {
    if chrome.title.is_empty() || painter.font().is_none() {
        return;
    }
    let pad = (TITLE_PADDING as f32 * m.scale).round() as i32;
    let gap = m.unit.max(1);

    let text_start = slots.close.right() + gap + pad;
    let right_x = slots
        .minimize
        .or(slots.maximize)
        .map(|r| r.x)
        .unwrap_or(m.titlebar.right());
    let text_end = right_x - gap - pad;
    let text_w = text_end - text_start;
    if text_w <= 0 {
        return;
    }

    let font_px = CHROME_FONT_SIZE * m.scale;
    let text_h = painter.measure_text(&chrome.title, font_px).h;
    let ty = m.titlebar.y + ((m.titlebar.h - text_h) / 2).max(0);

    let clip = painter.push_clip(Rect::new(text_start, m.titlebar.y, text_w, m.titlebar.h));
    painter.text(text_start, ty, &chrome.title, font_px, TITLEBAR_TEXT);
    painter.restore_clip(clip);
}

/// Draw the soft, rounded drop shadow over the desktop. Replicates Canoe's
/// quadratic-falloff rounded-rect shadow with the falloff curve
/// `alpha = base · (1 − d/s)²` out to `s`.
///
/// The occluder that casts the shadow is the window frame with its *top* edge
/// pushed down by half the shadow extent while its bottom and sides stay on the
/// frame (Canoe's `frame_height − shadow_size/2` height plus a `shadow_size/2`
/// downward shift). So the shadow hugs the bottom and side borders with no gap,
/// while the top stays light — the look of a window lit from above.
fn draw_shadow(painter: &mut Painter, m: &Metrics) {
    let s = m.shadow;
    if s <= 0 {
        return;
    }
    let s_f = s as f32;
    let shift = s / 2;
    // Occluder: frame top moved down by `shift`, bottom/left/right unchanged.
    let occ_x = m.frame.x;
    let occ_y = m.frame.y + shift;
    let occ_w = m.frame.w;
    let occ_h = (m.frame.h - shift).max(1);

    let cx = occ_x as f32 + occ_w as f32 / 2.0;
    let cy = occ_y as f32 + occ_h as f32 / 2.0;
    let hx = occ_w as f32 / 2.0;
    let hy = occ_h as f32 / 2.0;
    let r = (shift as f32).clamp(0.0, hx.min(hy).max(0.0));

    // Only the band around the occluder can carry shadow; scan that region.
    let x0 = (occ_x - s - 1).max(0);
    let x1 = (occ_x + occ_w + s + 1).min(m.buffer.w);
    let y0 = (occ_y - s - 1).max(0);
    let y1 = (occ_y + occ_h + s + 1).min(m.buffer.h);

    for py in y0..y1 {
        for px in x0..x1 {
            let dx = (px as f32 + 0.5) - cx;
            let dy = (py as f32 + 0.5) - cy;
            // Signed distance to the rounded rect.
            let qx = dx.abs() - (hx - r);
            let qy = dy.abs() - (hy - r);
            let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
            let inside = qx.max(qy).min(0.0);
            let dist = outside + inside - r;
            if dist <= 0.0 || dist > s_f {
                continue;
            }
            let falloff = 1.0 - dist / s_f;
            let alpha = (SHADOW_ALPHA * falloff * falloff).round();
            if alpha <= 0.0 {
                continue;
            }
            painter.blend_pixel_phys(px, py, Color::BLACK, alpha as u8);
        }
    }
}
