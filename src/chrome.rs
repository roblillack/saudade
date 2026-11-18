//! Window chrome for framed screenshots, and for the windows of a compositor
//! that leaves decorating them to us.
//!
//! On a real desktop the title bar and frame are usually the window manager's
//! job. For automated screenshots, though, it is often nice to capture a window
//! the way a user actually sees it: sitting on the desktop, inside a title bar
//! and frame, with a drop shadow. This module draws that chrome around a
//! rendered widget tree. The Wayland runtime draws the same chrome around its
//! own windows where the compositor wants client-side decorations (GNOME's,
//! for one), so a saudade window looks the same there as it does on Canoe.
//!
//! The look reproduces the default rendering style of **Canoe** (the Win 3.1
//! styled stacking window manager saudade is usually paired with): a teal
//! desktop, a soft drop shadow, a navy active title bar, light-gray beveled
//! window controls, and a black multi-layer border. Windows are drawn in their
//! *active* (focused) state unless [`WindowChrome::inactive`] says otherwise.
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
//! Drive it through [`MockBackend::render_framed`](crate::mock::MockBackend::render_framed),
//! or arrange several windows on one [`Desktop`](crate::mock::Desktop).

use crate::font::{FontFamily, FontStyle};
use crate::geometry::{Color, Point, Rect, Size};
use crate::include_svg;
use crate::painter::Painter;
use crate::svg::SvgImage;

// --- Canoe default palette (its `UiConfig::default`), in saudade colors. ---

/// The teal Canoe desktop.
pub(crate) const DESKTOP_BACKGROUND: Color = Color::rgb(0x00, 0x80, 0x80);
/// Active title-bar fill (Win 3.1 navy).
const TITLEBAR_BG: Color = Color::NAVY;
/// Active title-bar text.
const TITLEBAR_TEXT: Color = Color::WHITE;
/// Inactive title-bar fill and text: black on white.
const TITLEBAR_BG_INACTIVE: Color = Color::WHITE;
const TITLEBAR_TEXT_INACTIVE: Color = Color::BLACK;
/// Outer/inner thin border layers, the separator, and the button dividers.
const BORDER_DARK: Color = Color::BLACK;
/// The bulk middle border layer for resizable / fixed windows.
const BORDER_MID: Color = Color::LIGHT_GRAY;
/// Window-control button face and its 3D bevel.
const BUTTON_BG: Color = Color::LIGHT_GRAY;
const BUTTON_HIGHLIGHT: Color = Color::WHITE;
const BUTTON_SHADOW: Color = Color::MID_GRAY;
/// The close box's face while it is held down.
const BUTTON_BG_PRESSED: Color = Color::rgb(0xA0, 0xA0, 0xA0);

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
pub(crate) const SHADOW_SIZE: i32 = 20;
/// Inactive-window soft-shadow extent.
const SHADOW_SIZE_INACTIVE: i32 = 10;
/// Default teal margin between the window (with its shadow) and the image edge.
const DEFAULT_MARGIN: i32 = 40;
/// Padding on each side of the title text within its slot.
const TITLE_PADDING: i32 = 6;
/// How far outside its frame a resizable window can still be grabbed by an
/// edge, in the room its drop shadow takes up.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
const RESIZE_GRAB: i32 = 6;

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

    /// The bulk middle border layer color. Dialogs blend it into the title bar,
    /// filled `titlebar_bg`.
    fn mid_color(self, titlebar_bg: Color) -> Color {
        match self {
            WindowFrame::Dialog => titlebar_bg,
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
    active: bool,
    desktop_background: Color,
    margin: i32,
    /// Whether the window is free to move and resize: a maximized or tiled
    /// one is neither, and casts no shadow.
    floating: bool,
    /// Whether the minimize / maximize buttons a frame style has are shown:
    /// a compositor may offer neither.
    can_minimize: bool,
    can_maximize: bool,
    /// The title-bar button drawn held down.
    pressed: Option<TitleButton>,
}

impl WindowChrome {
    /// Chrome with the given window `title` and frame style, drawn active, on
    /// the default teal desktop with the default margin.
    pub fn new(title: impl Into<String>, frame: WindowFrame) -> Self {
        Self {
            title: title.into(),
            frame,
            active: true,
            desktop_background: DESKTOP_BACKGROUND,
            margin: DEFAULT_MARGIN,
            floating: true,
            can_minimize: true,
            can_maximize: true,
            pressed: None,
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

    /// Draw the window *inactive*, as one without the focus: black title text
    /// on a white bar, and a drop shadow half the size. For a screenshot of
    /// several windows on a [`Desktop`](crate::mock::Desktop), where only one
    /// of them can be the active window.
    pub fn inactive(mut self) -> Self {
        self.active = false;
        self
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

    /// Draw the window active or not — [`Self::inactive`], for a window whose
    /// focus comes and goes.
    #[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
    pub(crate) fn with_active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// A window that is maximized or tiled: no drop shadow, and no edges to
    /// resize it by.
    #[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
    pub(crate) fn docked(mut self, docked: bool) -> Self {
        self.floating = !docked;
        self
    }

    /// Leave out the minimize and / or maximize button the frame style has,
    /// for a compositor that can't do what it does.
    #[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
    pub(crate) fn with_buttons(mut self, minimize: bool, maximize: bool) -> Self {
        self.can_minimize = minimize;
        self.can_maximize = maximize;
        self
    }

    /// Draw `button` held down.
    #[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
    pub(crate) fn with_pressed(mut self, button: Option<TitleButton>) -> Self {
        self.pressed = button;
        self
    }

    /// Logical px the frame adds on each side of the content: the border all
    /// round, and the title bar with the line beneath it on top.
    #[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
    pub(crate) fn frame_insets(&self) -> Insets {
        let bw = self.frame.border_width();
        Insets {
            left: bw,
            top: bw + titlebar_height() + 1,
            right: bw,
            bottom: bw,
        }
    }

    /// Logical px the drop shadow reaches past the frame on each side — the
    /// active window's, the larger, so that the room a live window keeps for
    /// it doesn't change with the focus. Nothing for a window that casts none.
    /// The shadow falls down and away from a light above, so it reaches only
    /// half as far above the window as below it.
    #[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
    pub(crate) fn shadow_insets(&self) -> Insets {
        if !self.floating {
            return Insets::default();
        }
        Insets {
            left: SHADOW_SIZE,
            top: SHADOW_SIZE - SHADOW_SIZE / 2,
            right: SHADOW_SIZE,
            bottom: SHADOW_SIZE,
        }
    }

    /// Logical px outside the frame its edges can be grabbed by: some, for a
    /// resizable window that isn't maximized or tiled, else none.
    #[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
    pub(crate) fn resize_grab(&self) -> i32 {
        if self.resizes_by_edges() {
            RESIZE_GRAB
        } else {
            0
        }
    }

    fn resizes_by_edges(&self) -> bool {
        self.floating && self.frame == WindowFrame::Resizable
    }

    /// Whether a window in this frame style paints the desktop background
    /// pattern behind its content. Mirrors the live backend: a regular
    /// top-level window (resizable / fixed) honors the pattern, while a dialog
    /// — rendered in the live runtime's separate popup/dialog pass — stays
    /// plain.
    pub(crate) fn paints_background_pattern(&self) -> bool {
        self.frame != WindowFrame::Dialog
    }

    pub(crate) fn desktop_background(&self) -> Color {
        self.desktop_background
    }

    /// The title bar's fill and text color.
    fn titlebar_colors(&self) -> (Color, Color) {
        if self.active {
            (TITLEBAR_BG, TITLEBAR_TEXT)
        } else {
            (TITLEBAR_BG_INACTIVE, TITLEBAR_TEXT_INACTIVE)
        }
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
    pub frame: Rect,
    /// The title-bar strip (inside the border, above the separator).
    titlebar: Rect,
    /// Where the content blits in: top-left and (== the rendered content) size.
    pub content: Rect,
    /// Resize-border thickness.
    bw: i32,
    /// Separator / thin-line / divider thickness (one logical px).
    unit: i32,
    /// How deep one and two logical px of a title-bar button's bevel run:
    /// `unit` and twice it, except from 1.5x up to 1.75x, where the bevel is
    /// drawn at 1.5x as the content's own buttons' is — 2 and 3.
    bevel: (i32, i32),
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
    let unit = crate::painter::chrome_px(scale);
    let margin = px(chrome.margin);

    let cw = content_phys.w.max(1);
    let ch = content_phys.h.max(1);

    // Border + title bar + 1px separator + content + border.
    let frame_w = cw + bw * 2;
    let frame_h = ch + tbh + unit + bw * 2;

    let buffer = Size::new(frame_w + margin * 2, frame_h + margin * 2);
    let frame = Rect::new(margin, margin, frame_w, frame_h);
    let content = Rect::new(frame.x + bw, frame.y + bw + tbh + unit, cw, ch);
    metrics_around(frame, content, buffer, scale, chrome)
}

/// The geometry of a frame already placed: `frame` and the `content` it holds,
/// in a `buffer` that has room for them (all physical px), drawn at `scale`.
///
/// The top border is the thickness the frame style has at `scale`, and the
/// title bar takes what is left above the content after it and the line
/// beneath; the other three borders are whatever `content` leaves of `frame`.
/// That is the same frame [`metrics`] lays out, for a window whose frame and
/// content are placed by rounding a size the compositor measures in its own
/// units, where a border may come out a pixel thicker on one side than the
/// other.
pub(crate) fn metrics_around(
    frame: Rect,
    content: Rect,
    buffer: Size,
    scale: f32,
    chrome: &WindowChrome,
) -> Metrics {
    let scale = scale.max(0.01);
    let px = |logical: i32| (logical as f32 * scale).round() as i32;

    let bw = px(chrome.frame.border_width());
    // Thin lines are a whole number of device pixels, the same as the frames
    // the content's own widgets draw.
    let unit = crate::painter::chrome_px(scale);
    let bevel = (
        crate::painter::bevel_depth(1, scale),
        crate::painter::bevel_depth(2, scale),
    );
    let shadow = if !chrome.floating {
        0
    } else if chrome.active {
        px(SHADOW_SIZE)
    } else {
        px(SHADOW_SIZE_INACTIVE)
    };

    let titlebar_y = frame.y + bw;
    let titlebar = Rect::new(
        content.x,
        titlebar_y,
        content.w,
        (content.y - unit - titlebar_y).max(0),
    );

    Metrics {
        scale,
        buffer,
        frame,
        titlebar,
        content,
        bw,
        unit,
        bevel,
        shadow,
    }
}

/// Paint the shadow, border, title bar, buttons, and title into the painter
/// (which draws at scale 1.0, in [`Metrics::buffer`]'s coordinates). The
/// shadow falls over whatever the painter already holds — the desktop. The
/// content area is left untouched, for the caller to blit the rendered widget
/// tree into, or to have painted it already.
pub(crate) fn paint_window(painter: &mut Painter, m: &Metrics, chrome: &WindowChrome) {
    draw_shadow(painter, m);

    // Border ring(s), around the title bar, the line beneath it, and the
    // content.
    let (titlebar_bg, _) = chrome.titlebar_colors();
    let inside = Rect::new(
        m.content.x,
        m.titlebar.y,
        m.content.w,
        m.content.bottom() - m.titlebar.y,
    );
    if chrome.frame == WindowFrame::Fixed {
        fill_ring(painter, m.frame, inside, BORDER_DARK);
    } else {
        let outer = m.frame.inset(m.unit);
        let inner = Rect::new(
            inside.x - m.unit,
            inside.y - m.unit,
            inside.w + 2 * m.unit,
            inside.h + 2 * m.unit,
        );
        fill_ring(painter, m.frame, outer, BORDER_DARK);
        fill_ring(painter, outer, inner, chrome.frame.mid_color(titlebar_bg));
        fill_ring(painter, inner, inside, BORDER_DARK);
    }

    // Title bar, then the black separator line beneath it.
    painter.fill_rect(m.titlebar, titlebar_bg);
    painter.fill_rect(
        Rect::new(m.titlebar.x, m.titlebar.bottom(), m.titlebar.w, m.unit),
        BORDER_DARK,
    );

    draw_buttons_and_title(painter, m, chrome);
}

/// [`paint_window`] for a live window, whose buffer is all its own: what lies
/// outside the frame is cleared to transparent first, for the shadow to fall
/// onto. The content is left alone, painted already.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub(crate) fn paint_decorations(painter: &mut Painter, m: &Metrics, chrome: &WindowChrome) {
    let all = Rect::new(0, 0, m.buffer.w, m.buffer.h);
    fill_ring(painter, all, m.frame, Color::TRANSPARENT);
    paint_window(painter, m, chrome);
}

/// Fill what lies inside `outer` but outside `inner`, which it holds.
fn fill_ring(painter: &mut Painter, outer: Rect, inner: Rect, color: Color) {
    let top = inner.y - outer.y;
    let bottom = outer.bottom() - inner.bottom();
    painter.fill_rect(Rect::new(outer.x, outer.y, outer.w, top), color);
    painter.fill_rect(Rect::new(outer.x, inner.bottom(), outer.w, bottom), color);
    painter.fill_rect(
        Rect::new(outer.x, inner.y, inner.x - outer.x, inner.h),
        color,
    );
    painter.fill_rect(
        Rect::new(
            inner.right(),
            inner.y,
            outer.right() - inner.right(),
            inner.h,
        ),
        color,
    );
}

/// A button in the title bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TitleButton {
    /// The control-menu box at the left: a double click closes the window.
    Close,
    Minimize,
    Maximize,
}

/// Logical px around something, one figure per side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub(crate) struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// The edges a resize handle moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub(crate) struct Edges {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

/// What a point of a decorated window's buffer lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub(crate) enum Hit {
    /// The window's content: the widget tree.
    Content,
    /// The title bar, or a border that doesn't resize the window: dragging it
    /// moves the window.
    Title,
    Button(TitleButton),
    /// A resize handle: the border of a resizable window, and a little of the
    /// shadow around it.
    Resize(Edges),
    /// The shadow, or past it: not the window at all.
    Outside,
}

/// What the physical-px point `p` of a framed buffer lands on. A corner of a
/// resizable window takes a title bar's height of each edge it joins, the
/// handle a diagonal resize is that much easier to find by.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub(crate) fn hit_test(m: &Metrics, chrome: &WindowChrome, p: Point) -> Hit {
    if m.content.contains(p) {
        return Hit::Content;
    }
    let f = m.frame;
    let grab = (chrome.resize_grab() as f32 * m.scale).round() as i32;
    let reach = Rect::new(f.x - grab, f.y - grab, f.w + 2 * grab, f.h + 2 * grab);
    if !reach.contains(p) {
        return Hit::Outside;
    }
    if chrome.resizes_by_edges() {
        let band = m.bw.max(1);
        let near = Edges {
            top: p.y < f.y + band,
            bottom: p.y >= f.bottom() - band,
            left: p.x < f.x + band,
            right: p.x >= f.right() - band,
        };
        if near.top || near.bottom || near.left || near.right {
            let corner = m.titlebar.h.max(band);
            let across = near.top || near.bottom;
            let along = near.left || near.right;
            return Hit::Resize(Edges {
                top: near.top || (along && p.y < f.y + corner),
                bottom: near.bottom || (along && p.y >= f.bottom() - corner),
                left: near.left || (across && p.x < f.x + corner),
                right: near.right || (across && p.x >= f.right() - corner),
            });
        }
    }
    let slots = button_slots(m, chrome);
    let buttons = [
        (Some(slots.close), TitleButton::Close),
        (slots.minimize, TitleButton::Minimize),
        (slots.maximize, TitleButton::Maximize),
    ];
    for (rect, button) in buttons {
        if rect.is_some_and(|r| r.contains(p)) {
            return Hit::Button(button);
        }
    }
    Hit::Title
}

/// The three title-bar button slots, laid out left-to-right then right-to-left
/// exactly as Canoe positions them: a close box at the far left, and the
/// maximize / minimize buttons hugging the right edge.
struct ButtonSlots {
    close: Rect,
    minimize: Option<Rect>,
    maximize: Option<Rect>,
}

fn button_slots(m: &Metrics, chrome: &WindowChrome) -> ButtonSlots {
    let frame = chrome.frame;
    let size = m.titlebar.h; // square buttons, one bar tall
    let y = m.titlebar.y;
    let close = Rect::new(m.titlebar.x, y, size, size);

    let gap = m.unit.max(1);
    let right_edge = m.titlebar.right();
    let mut next_right = right_edge - size;

    let maximize = (frame.shows_maximize() && chrome.can_maximize).then(|| {
        let r = Rect::new(next_right, y, size, size);
        next_right -= size + gap;
        r
    });
    let minimize = (frame.shows_minimize() && chrome.can_minimize)
        .then(|| Rect::new(next_right, y, size, size));

    ButtonSlots {
        close,
        minimize,
        maximize,
    }
}

fn draw_buttons_and_title(painter: &mut Painter, m: &Metrics, chrome: &WindowChrome) {
    let slots = button_slots(m, chrome);
    let unit = m.unit.max(1);
    let icon = (icon_size() as f32 * m.scale).round() as i32;
    let pressed = |button| chrome.pressed == Some(button);

    // Close box: a flat gray face with a black divider on its right, holding the
    // beveled Win 3.1 control-menu bar. Held down, the face darkens.
    let close_bg = if pressed(TitleButton::Close) {
        BUTTON_BG_PRESSED
    } else {
        BUTTON_BG
    };
    painter.fill_rect(slots.close, close_bg);
    draw_divider(painter, slots.close.right(), m, unit);
    CLOSE_ICON.draw(painter, icon_box(slots.close, icon, 0));

    // Held down, a beveled button sinks: its glyph moves down and right by
    // the depth of the bevel's first step.
    if let Some(min) = slots.minimize {
        let down = pressed(TitleButton::Minimize);
        draw_divider(painter, min.x - unit, m, unit);
        draw_button(painter, min, m.bevel, down);
        let nudge = if down { m.bevel.0 } else { 0 };
        MINIMIZE_ICON.draw(painter, icon_box(min, icon, nudge));
    }
    if let Some(max) = slots.maximize {
        let down = pressed(TitleButton::Maximize);
        draw_divider(painter, max.x - unit, m, unit);
        draw_button(painter, max, m.bevel, down);
        let nudge = if down { m.bevel.0 } else { 0 };
        MAXIMIZE_ICON.draw(painter, icon_box(max, icon, nudge));
    }

    draw_title(painter, m, &slots, chrome);
}

/// The centered `size`×`size` cell for a glyph inside a button, moved
/// `nudge` px down and right.
fn icon_box(button: Rect, size: i32, nudge: i32) -> Rect {
    Rect::new(
        button.x + (button.w - size) / 2 + nudge,
        button.y + (button.h - size) / 2 + nudge,
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
/// `one` and `two` are how deep its first and second logical px run:
/// [`Metrics::bevel`]. Held `down`, it is sunk instead: the face with a gray
/// shadow along its top and left, as Canoe draws it.
fn draw_button(painter: &mut Painter, r: Rect, (one, two): (i32, i32), down: bool) {
    painter.fill_rect(r, BUTTON_BG);
    if down {
        painter.fill_rect(Rect::new(r.x, r.y, r.w, one), BUTTON_SHADOW);
        painter.fill_rect(Rect::new(r.x, r.y, one, r.h), BUTTON_SHADOW);
        return;
    }
    // Highlight: top and left edges.
    painter.fill_rect(Rect::new(r.x, r.y, r.w, one), BUTTON_HIGHLIGHT);
    painter.fill_rect(Rect::new(r.x, r.y, one, r.h), BUTTON_HIGHLIGHT);
    // Inner shadow, from `one` in to `two`.
    if r.w >= one + two && r.h >= one + two {
        painter.fill_rect(
            Rect::new(r.x + one, r.bottom() - two, r.w - 2 * one, two - one),
            BUTTON_SHADOW,
        );
        painter.fill_rect(
            Rect::new(r.right() - two, r.y + one, two - one, r.h - 2 * one),
            BUTTON_SHADOW,
        );
    }
    // Outer shadow: bottom and right edges.
    painter.fill_rect(Rect::new(r.x, r.bottom() - one, r.w, one), BUTTON_SHADOW);
    painter.fill_rect(Rect::new(r.right() - one, r.y, one, r.h), BUTTON_SHADOW);
}

/// Draw the title text in bold: centered on the whole title bar, as Canoe
/// draws it, and vertically centered in the bar. A title that would run into
/// the buttons there moves aside just far enough to clear them, and one too
/// wide for the space between them starts at its left and is clipped.
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
    let size =
        painter.measure_text_styled(&chrome.title, font_px, FontFamily::Sans, FontStyle::Bold);
    let tx = centered_title_x(m.titlebar, text_start, text_end, size.w);
    let ty = m.titlebar.y + ((m.titlebar.h - size.h) / 2).max(0);

    let clip = painter.push_clip(Rect::new(text_start, m.titlebar.y, text_w, m.titlebar.h));
    let (_, text_color) = chrome.titlebar_colors();
    painter.text_styled(
        tx,
        ty,
        &chrome.title,
        font_px,
        text_color,
        FontFamily::Sans,
        FontStyle::Bold,
    );
    painter.restore_clip(clip);
}

/// Where a title `width` wide starts: centered on the whole `titlebar`, kept
/// within `start..end`, the space between the buttons — or at `start`, if it
/// is wider than that.
fn centered_title_x(titlebar: Rect, start: i32, end: i32, width: i32) -> i32 {
    if width > end - start {
        return start;
    }
    (titlebar.x + (titlebar.w - width) / 2).clamp(start, end - width)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontSet;

    fn at(x: i32, y: i32) -> Point {
        Point::new(x, y)
    }

    /// A 200×100 window framed at a scale of 1, with the default 40 px margin:
    /// the frame spans 40..248 across and 40..168 down, the title bar
    /// 44..63, and the content starts at (44, 64).
    fn framed(chrome: &WindowChrome) -> Metrics {
        metrics(Size::new(200, 100), 1.0, chrome)
    }

    #[test]
    fn a_framed_window_is_laid_out_inside_its_margin() {
        let m = framed(&WindowChrome::resizable("x"));
        assert_eq!(m.frame, Rect::new(40, 40, 208, 128));
        assert_eq!(m.titlebar, Rect::new(44, 44, 200, 19));
        assert_eq!(m.content, Rect::new(44, 64, 200, 100));
        assert_eq!(m.buffer, Size::new(288, 208));
    }

    #[test]
    fn points_land_on_what_is_drawn_there() {
        let chrome = WindowChrome::resizable("x");
        let m = framed(&chrome);
        assert_eq!(hit_test(&m, &chrome, at(140, 110)), Hit::Content);
        assert_eq!(hit_test(&m, &chrome, at(140, 50)), Hit::Title);
        assert_eq!(
            hit_test(&m, &chrome, at(50, 50)),
            Hit::Button(TitleButton::Close)
        );
        assert_eq!(
            hit_test(&m, &chrome, at(240, 50)),
            Hit::Button(TitleButton::Maximize)
        );
        assert_eq!(
            hit_test(&m, &chrome, at(220, 50)),
            Hit::Button(TitleButton::Minimize)
        );
        assert_eq!(hit_test(&m, &chrome, at(5, 5)), Hit::Outside);
    }

    #[test]
    fn a_resizable_window_resizes_by_its_border_and_a_little_beyond() {
        let chrome = WindowChrome::resizable("x");
        let m = framed(&chrome);
        let left = Edges {
            left: true,
            ..Edges::default()
        };
        assert_eq!(hit_test(&m, &chrome, at(41, 110)), Hit::Resize(left));
        assert_eq!(hit_test(&m, &chrome, at(36, 110)), Hit::Resize(left));
        assert_eq!(hit_test(&m, &chrome, at(30, 110)), Hit::Outside);
        // Near a corner, both edges.
        let corner = Edges {
            bottom: true,
            right: true,
            ..Edges::default()
        };
        assert_eq!(hit_test(&m, &chrome, at(247, 160)), Hit::Resize(corner));
        assert_eq!(hit_test(&m, &chrome, at(240, 167)), Hit::Resize(corner));
    }

    #[test]
    fn a_fixed_or_docked_window_does_not_resize() {
        let fixed = WindowChrome::fixed("x");
        let m = framed(&fixed);
        assert_eq!(hit_test(&m, &fixed, at(40, 110)), Hit::Title);
        assert_eq!(hit_test(&m, &fixed, at(38, 110)), Hit::Outside);

        let docked = WindowChrome::resizable("x").docked(true);
        let m = framed(&docked);
        assert_eq!(m.shadow, 0);
        assert_eq!(hit_test(&m, &docked, at(41, 110)), Hit::Title);
    }

    #[test]
    fn buttons_the_compositor_cannot_honor_are_left_out() {
        let chrome = WindowChrome::resizable("x").with_buttons(true, false);
        let m = framed(&chrome);
        // The minimize button moves up against the right edge.
        assert_eq!(
            hit_test(&m, &chrome, at(240, 50)),
            Hit::Button(TitleButton::Minimize)
        );
        assert_eq!(hit_test(&m, &chrome, at(220, 50)), Hit::Title);
    }

    #[test]
    fn a_title_is_centered_on_the_whole_bar_and_kept_clear_of_the_buttons() {
        let bar = Rect::new(0, 0, 400, 20);
        // Close box up to 30, buttons from 350: room for 100 in the middle.
        assert_eq!(centered_title_x(bar, 30, 350, 100), 150);
        // Room on the left is short, so it moves right just far enough.
        assert_eq!(centered_title_x(bar, 180, 350, 100), 180);
        // And the other way.
        assert_eq!(centered_title_x(bar, 30, 220, 100), 120);
        // Too wide for the space: from its left, to be clipped.
        assert_eq!(centered_title_x(bar, 30, 120, 100), 30);
    }

    /// A live window's content is painted before its decorations, which must
    /// leave it be; and what lies outside the frame is see-through but for
    /// the shadow.
    #[test]
    fn decorations_leave_the_content_alone_and_clear_around_the_frame() {
        let chrome = WindowChrome::resizable("x");
        let m = framed(&chrome);
        let sentinel = Color::rgb(0x12, 0x34, 0x56);
        let mut pixels = vec![sentinel.0; (m.buffer.w * m.buffer.h) as usize];
        let fonts = FontSet {
            sans: None,
            serif: None,
            mono: None,
        };
        {
            let mut painter = Painter::new(&mut pixels, m.buffer.w, m.buffer.h, 1.0, 0, 0, fonts);
            paint_decorations(&mut painter, &m, &chrome);
        }
        let px = |x: i32, y: i32| Color(pixels[(y * m.buffer.w + x) as usize]);
        let c = m.content;
        for (x, y) in [
            (c.x, c.y),
            (c.right() - 1, c.bottom() - 1),
            (c.x + 50, c.y + 50),
        ] {
            assert_eq!(px(x, y), sentinel);
        }
        assert_eq!(px(1, 1), Color::TRANSPARENT);
        assert_eq!(px(m.frame.x, m.frame.y), BORDER_DARK);
        // Below the window, in its shadow: black, partly see-through.
        let shade = px(c.x + 50, m.frame.bottom() + 2);
        assert!(shade.alpha() > 0 && shade.alpha() < 255);
    }
}
