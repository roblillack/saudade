//! scaling — preview saudade widgets at an arbitrary logical→physical scale.
//!
//! The window's own scale factor belongs to the OS: saudade adopts whatever
//! the compositor reports at startup and refreshes it on a `ScaleFactorChanged`
//! event, and there is deliberately no API for a widget to override it. What a
//! widget *can* do is render content at a scale of its choosing through
//! [`Painter::draw_scaled`] — it paints the content as a real window opened at
//! that DPI would (chrome snapped to device pixels, text re-rasterized at its
//! physical size, nothing resampled), into a region of the surface.
//!
//! This demo wires a [`Slider`] and two rows of preset [`Button`]s to a shared
//! "preview scale" factor, then hands it to a `ScalePreview` widget that draws
//! a small panel of real widgets — a text input, a dropdown, a checkbox,
//! buttons (one of them focused, for its dotted focus rectangle), a progress
//! bar, a scrollbar — at that scale, in a canvas below the controls. The slider
//! starts at this display's actual OS scale, so the panel opens looking exactly
//! like the rest of the window.
//!
//! The factor is the *absolute* logical→physical scale, the same number the OS
//! reports. Try the fractional steps (1.25x, 1.5x): that's where saudade's
//! crisp physical-pixel chrome pass earns its keep. The presets walk the ladder
//! the runtime itself picks from — the factor itself on Windows and X11, an
//! eighth over it on a Mac (2.25x on a Retina one) — up into the range where a
//! logical pixel is worth two-and-a-fraction physical ones and every edge has
//! to round.
//!
//! Two checkboxes along the bottom pick how the rendered panel reaches the
//! screen, and they are the two halves of what a scale factor means:
//!
//! * **Zoom in 2x** magnifies the *rendered result* 2× (a pure pixel copy — it
//!   does not re-run the scaling at a higher factor) so you can see the
//!   per-pixel snapping a scale produced.
//! * **Scale to fit** pins the panel to the size of the canvas and resamples
//!   the render into it ([`Painter::draw_resampled`]), so dragging the slider
//!   holds the panel's size on screen and changes only how many device pixels
//!   it was drawn from — which is precisely what swapping the display under a
//!   window for a denser one of the same physical size does. Zoom is redundant
//!   there (the fit is already a magnification), so it greys out.
//!
//! The window is yours to size, and nothing here ever resizes it out from under
//! you: it opens at the room the panel needs on this display and then stays
//! where you leave it, the controls and the canvas reflowing to whatever it
//! becomes. What the two modes differ in is what a panel too big for the canvas
//! does. A free-size preview is centered and clipped, so a high factor in a
//! small window shows the middle of the panel at full detail — drag the window
//! out to see the rest. A fitted one shrinks instead, and never spills.
//!
//! A status bar along the bottom reports the window's *actual* OS scale factor
//! (`Painter::system_scale`) — independent of the preview slider above — and,
//! when it differs, the scale the window is really being drawn at. It differs
//! on Wayland, where a fractional display (say 150%) is oversampled at 2.0x and
//! resampled down by the compositor; on macOS, where saudade corrects the factor
//! for a point that is nearer 1/108 in than the 96-dpi pixel it draws in; and
//! anywhere `SAUDADE_UI_DPI` has moved that pixel.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use saudade::{
    App, Button, Checkbox, Color, Container, Dropdown, Event, EventCtx, Label, Painter,
    PopupRequest, ProgressBar, Rect, SCROLLBAR_THICKNESS, ScrollBar, Size, Slider, TextInput,
    Theme, Widget, WindowConfig,
};

/// Layout metrics. The controls occupy a fixed-height band at the top (down to
/// `CANVAS_Y`) and need at least `MIN_W` to lay out; the toggle row and the
/// status bar occupy fixed bands at the bottom; the preview canvas is
/// everything in between, and takes up whatever slack a resize leaves.
const MIN_W: i32 = 480;
const CANVAS_X: i32 = 24;
const PANEL_PAD: i32 = 24;
/// Smallest canvas the layout bothers with — the floor the window's own minimum
/// size is derived from, and what the canvas clamps to if a backend hands us a
/// window below it anyway.
const MIN_CANVAS: i32 = 80;
/// Height of the bottom status bar that reports the real OS scale factor.
const FOOTER_H: i32 = 24;
/// The row of checkboxes above the status bar, and the air around it.
const TOGGLE_H: i32 = 16;
const TOGGLE_GAP: i32 = 10;

/// The preset grid: `PRESET_ROWS` rows of `PRESET_COLS` buttons, starting at
/// `PRESET_Y`. The canvas derives its own y from these, so adding a row of
/// presets moves it down.
const PRESET_W: i32 = 80;
const PRESET_H: i32 = 24;
const PRESET_COLS: i32 = 5;
const PRESET_ROWS: i32 = 2;
const PRESET_Y: i32 = 138;
const PRESET_ROW_GAP: i32 = 4;
/// First free row under the preset grid.
const PRESETS_BOTTOM: i32 = PRESET_Y + PRESET_ROWS * PRESET_H + (PRESET_ROWS - 1) * PRESET_ROW_GAP;
/// Top of the preview canvas.
const CANVAS_Y: i32 = PRESETS_BOTTOM + 12;
/// Smallest window the layout stays legible in: the fixed bands plus the
/// smallest canvas worth drawing into. The runtime hands it to the window
/// manager, so nothing below this ever reaches `layout`.
const MIN_H: i32 = CANVAS_Y + MIN_CANVAS + 2 * TOGGLE_GAP + TOGGLE_H + FOOTER_H;

/// Slider range, in percent (100% = 1.0x … 350% = 3.5x). The top reaches the
/// densest preset rather than a round number, so every preset is a position the
/// slider can also be dragged to.
const MIN_PCT: i32 = 100;
const MAX_PCT: i32 = 350;
/// Preset scales, ascending, filling the grid row by row: the first row runs
/// 1.0x to 2.0x and the second carries on to 3.0x, with a 3.5x at the end.
///
/// Every one is a quarter step, which is the ladder Windows and X11 hand over
/// and saudade passes through: 1.0x is 100%, 1.25x is 125%, 2.0x is 200% (see
/// `saudade::ui_scale`). A Retina Mac's 2.25x is on it too, an eighth over its
/// factor of 2; the 3.5x at the end is headroom, a display denser than any of
/// them, to see the chrome hold its geometry.
const PRESETS: [(&str, i32); 10] = [
    ("1.0x", 100),
    ("1.25x", 125),
    ("1.5x", 150),
    ("1.75x", 175),
    ("2.0x", 200),
    ("2.25x", 225),
    ("2.5x", 250),
    ("2.75x", 275),
    ("3.0x", 300),
    ("3.5x", 350),
];

/// The sample panel's natural footprint, in *preview-logical* pixels. The
/// widgets sit a few pixels inside it (see [`build_sample`]), so the slack
/// absorbs any rounding when the panel is clipped to the canvas.
const SAMPLE_W: i32 = 150;
const SAMPLE_H: i32 = 172;
/// The sample panel's extent as a [`Size`], for [`Painter::draw_resampled`].
const SAMPLE: Size = Size {
    w: SAMPLE_W,
    h: SAMPLE_H,
};

/// The sample panel's on-screen footprint (after the optional 2× zoom) in the
/// window's logical pixels, given the OS scale `os_scale`. This is the literal
/// space `draw_scaled` will fill: the panel is rendered at `factor` and then
/// magnified by `zoom`, so its physical size is `SAMPLE × factor × zoom`, which
/// divided by `os_scale` gives logical pixels.
///
/// Only the free-size mode is measured this way. Under "scale to fit" the
/// footprint is the canvas' to dictate, not the factor's — see [`fit_area`].
fn footprint(factor: f32, zoom: bool, os_scale: f32) -> (i32, i32) {
    let z = if zoom { 2.0 } else { 1.0 };
    let s = os_scale.max(0.01);
    let w = (SAMPLE_W as f32 * factor * z / s).round().max(1.0) as i32;
    let h = (SAMPLE_H as f32 * factor * z / s).round().max(1.0) as i32;
    (w, h)
}

/// Window size that fits a panel of footprint `fw × fh`: the canvas pads the
/// panel by `PANEL_PAD` on each side (and is inset `CANVAS_X` from the window
/// edges), and the width is floored at the controls' `MIN_W`.
///
/// Only the *opening* size is chosen this way, for the panel at its natural
/// size. From then on the window is the user's, and it is the preview that
/// gives when the two disagree.
fn window_for_footprint(fw: i32, fh: i32) -> (i32, i32) {
    let w = (fw + 2 * (PANEL_PAD + CANVAS_X)).max(MIN_W);
    let h = CANVAS_Y + fh + 2 * PANEL_PAD + 2 * TOGGLE_GAP + TOGGLE_H + FOOTER_H;
    (w, h)
}

/// The slider spans from a fixed left edge out to the right margin of a window
/// `w` wide.
fn slider_rect(w: i32) -> Rect {
    Rect::new(140, 92, w - 164, 22)
}
/// The heading, and the paragraph under it. Both span the width so the
/// paragraph re-wraps instead of spilling when the window narrows.
fn title_rect(w: i32) -> Rect {
    Rect::new(24, 16, w - 48, 18)
}
fn intro_rect(w: i32) -> Rect {
    Rect::new(24, 38, w - 48, 42)
}
/// Preset button `i`, filling the grid left to right and top to bottom. Each
/// row spreads evenly across a window `w` wide, keeping a fixed button width
/// and growing the gaps.
fn preset_rect(i: i32, w: i32) -> Rect {
    let (row, col) = (i / PRESET_COLS, i % PRESET_COLS);
    let gap = ((w - 48 - PRESET_COLS * PRESET_W) / (PRESET_COLS - 1)).max(0);
    Rect::new(
        24 + col * (PRESET_W + gap),
        PRESET_Y + row * (PRESET_H + PRESET_ROW_GAP),
        PRESET_W,
        PRESET_H,
    )
}
/// The right-hand ("3.5x") slider tick, pinned under the slider's right end.
fn max_tick_rect(w: i32) -> Rect {
    Rect::new(w - 64, 118, 40, 14)
}

/// The toggle row, pinned above the status bar of a window `h` tall: the zoom
/// checkbox, then the fit one beside it.
fn toggles_y(h: i32) -> i32 {
    h - FOOTER_H - TOGGLE_GAP - TOGGLE_H
}
fn zoom_rect(h: i32) -> Rect {
    Rect::new(CANVAS_X, toggles_y(h), 110, TOGGLE_H)
}
fn fit_rect(h: i32) -> Rect {
    Rect::new(CANVAS_X + 130, toggles_y(h), 130, TOGGLE_H)
}

/// The preview canvas: everything between the controls and the toggle row, in a
/// window `w × h`. The window is resizable, so this is the piece that absorbs
/// the slack — every other band keeps its height.
fn canvas_rect(w: i32, h: i32) -> Rect {
    Rect::new(
        CANVAS_X,
        CANVAS_Y,
        (w - 2 * CANVAS_X).max(MIN_CANVAS),
        (toggles_y(h) - TOGGLE_GAP - CANVAS_Y).max(MIN_CANVAS),
    )
}

/// A `w × h` rectangle centered in `area` — which is where the panel goes in
/// both modes, and lands outside `area` on the axes where it doesn't fit, for
/// the caller's clip to trim.
fn centered(area: Rect, w: i32, h: i32) -> Rect {
    Rect::new(area.x + (area.w - w) / 2, area.y + (area.h - h) / 2, w, h)
}

/// The panel's on-screen box under "scale to fit": the largest rectangle with
/// the sample's proportions that fits inside `content` with `FIT_PAD` to spare.
///
/// It is a function of the canvas alone — the scale factor is deliberately not
/// an input. That is the whole point of the mode: the box holds still while the
/// factor moves, so the slider changes only how many device pixels the panel is
/// rendered from, the way a denser display of the same size would.
const FIT_PAD: i32 = 12;
fn fit_area(content: Rect) -> Rect {
    let avail_w = (content.w - 2 * FIT_PAD).max(8) as f32;
    let avail_h = (content.h - 2 * FIT_PAD).max(8) as f32;
    let k = (avail_w / SAMPLE_W as f32).min(avail_h / SAMPLE_H as f32);
    let w = (SAMPLE_W as f32 * k).round().max(1.0) as i32;
    let h = (SAMPLE_H as f32 * k).round().max(1.0) as i32;
    centered(content, w, h)
}

/// A label the root repositions on every resize, paired with the rule that
/// gives its rect in a window `w` wide.
type FlexLabel = (Rc<RefCell<Label>>, fn(i32) -> Rect);

fn main() {
    // Shared state. `factor` is the scale the preview renders at — 0.0 until the
    // first paint adopts the OS scale (see `Root::paint`). `zoom` is the 2×
    // magnify toggle and `fit` the scale-to-fit one; `win` caches the window's
    // logical size, refreshed by the root on every layout, so the inert widgets
    // can place themselves without a painter in hand.
    let factor = Rc::new(Cell::new(0.0_f32));
    let zoom = Rc::new(Cell::new(false));
    let fit = Rc::new(Cell::new(false));

    // The window opens at the default scale (factor == OS scale), where the
    // panel renders at its natural `SAMPLE` size regardless of the display.
    // Every size after that one is the user's.
    let (init_w, init_h) = window_for_footprint(SAMPLE_W, SAMPLE_H);
    let win = Rc::new(Cell::new(Size::new(init_w, init_h)));

    // The width-spanning controls are shared so the root can reposition them in
    // `layout` when a resize changes the width — the same instances the
    // container routes events and paints through. So are the two checkboxes,
    // which hang off the *bottom* edge and move with the height.
    let slider = Rc::new(RefCell::new(
        Slider::new(slider_rect(init_w), MIN_PCT, MAX_PCT)
            .with_step(5)
            .on_change({
                let factor = factor.clone();
                move |_, pct| factor.set(pct as f32 / 100.0)
            }),
    ));
    let labels: Vec<FlexLabel> = vec![
        (
            Rc::new(RefCell::new(
                Label::new(title_rect(init_w), "Scale factor preview").with_size(13.0),
            )),
            title_rect,
        ),
        (
            Rc::new(RefCell::new(
                Label::new(
                    intro_rect(init_w),
                    "Render a panel of widgets at any logical-to-physical scale — the window's\n\
                     own scale never changes. Zoom magnifies the rendered result 2x to reveal\n\
                     pixels; scale to fit holds the panel's size while the scale changes under it.",
                )
                .with_size(10.0),
            )),
            intro_rect,
        ),
        (
            Rc::new(RefCell::new(
                Label::new(max_tick_rect(init_w), "3.5x")
                    .with_size(9.0)
                    .with_color(Color::DARK_GRAY),
            )),
            max_tick_rect,
        ),
    ];
    let presets: Vec<Rc<RefCell<Button>>> = PRESETS
        .iter()
        .enumerate()
        .map(|(i, &(label, pct))| {
            let button = Button::new(preset_rect(i as i32, init_w), label).on_click({
                let slider = slider.clone();
                let factor = factor.clone();
                move |_| {
                    slider.borrow_mut().set_value(pct);
                    factor.set(pct as f32 / 100.0);
                }
            });
            Rc::new(RefCell::new(button))
        })
        .collect();

    let zoom_box = Rc::new(RefCell::new(
        Checkbox::new(zoom_rect(init_h), "Zoom in 2x").on_toggle({
            let zoom = zoom.clone();
            move |_, on| zoom.set(on)
        }),
    ));
    // Fitting the preview to the window takes the zoom out of play: the fit is
    // already a magnification, and it is the canvas that decides how much of
    // one. The checkbox keeps its setting while it greys out, and gets it back
    // when the fit is switched off.
    let fit_box = Rc::new(RefCell::new(
        Checkbox::new(fit_rect(init_h), "Scale to fit").on_toggle({
            let fit = fit.clone();
            let zoom_box = zoom_box.clone();
            move |_, on| {
                fit.set(on);
                zoom_box.borrow_mut().set_enabled(!on);
            }
        }),
    ));

    let mut body = Container::new(init_w, init_h);
    for (label, _) in &labels {
        body.push(Shared(label.clone()));
    }
    body.push(FactorReadout::new(
        Rect::new(24, 88, 110, 34),
        factor.clone(),
        zoom.clone(),
        fit.clone(),
    ));
    body.push(Label::new(Rect::new(140, 118, 40, 14), "1.0x").with_size(9.0));
    body.push(Shared(slider.clone()));
    for preset in &presets {
        body.push(Shared(preset.clone()));
    }
    body.push(ScalePreview::new(
        factor.clone(),
        zoom.clone(),
        fit.clone(),
        win.clone(),
    ));
    body.push(Shared(zoom_box.clone()));
    body.push(Shared(fit_box.clone()));
    body.push(StatusBar { win: win.clone() });

    App::new(
        WindowConfig::new("Scale Factor", init_w, init_h)
            .resizable(true)
            .min_size(MIN_W, MIN_H),
        Root::new(
            body,
            factor.clone(),
            win,
            slider,
            labels,
            presets,
            zoom_box,
            fit_box,
        ),
    )
    .with_theme(Theme::windows_31())
    .run();
}

/// Root wrapper. It lets the content fill the window instead of being centered
/// at a fixed design size (it reports its allocated bounds, so the runtime never
/// letterboxes, and floods them white), reflows the controls that follow an
/// edge when the window resizes, caches the window's size for the widgets that
/// place themselves against it, and owns the first-paint bootstrap — then defers
/// everything else to the inner [`Container`].
struct Root {
    inner: Container,
    bounds: Rect,
    factor: Rc<Cell<f32>>,
    win: Rc<Cell<Size>>,
    // The controls that follow an edge, repositioned on every `layout`: the
    // width-spanning band at the top, and the toggles pinned to the bottom.
    slider: Rc<RefCell<Slider>>,
    labels: Vec<FlexLabel>,
    presets: Vec<Rc<RefCell<Button>>>,
    zoom_box: Rc<RefCell<Checkbox>>,
    fit_box: Rc<RefCell<Checkbox>>,
}

impl Root {
    #[allow(clippy::too_many_arguments)]
    fn new(
        inner: Container,
        factor: Rc<Cell<f32>>,
        win: Rc<Cell<Size>>,
        slider: Rc<RefCell<Slider>>,
        labels: Vec<FlexLabel>,
        presets: Vec<Rc<RefCell<Button>>>,
        zoom_box: Rc<RefCell<Checkbox>>,
        fit_box: Rc<RefCell<Checkbox>>,
    ) -> Self {
        let (w, h) = window_for_footprint(SAMPLE_W, SAMPLE_H);
        Self {
            inner,
            bounds: Rect::new(0, 0, w, h),
            factor,
            win,
            slider,
            labels,
            presets,
            zoom_box,
            fit_box,
        }
    }
}

impl Widget for Root {
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        // First paint: adopt the OS scale as the starting preview scale and move
        // the slider thumb to match, before any child reads either.
        if self.factor.get() <= 0.0 {
            let os = painter.scale();
            self.factor.set(os);
            self.slider
                .borrow_mut()
                .set_value((os * 100.0).round() as i32);
        }
        painter.fill_rect(self.bounds, Color::WHITE);
        self.inner.paint(painter, theme);
    }
    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        self.inner.paint_overlay(painter, theme);
    }
    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.inner.event(event, ctx);
    }
    fn captures_pointer(&self) -> bool {
        self.inner.captures_pointer()
    }
    fn focusable(&self) -> bool {
        self.inner.focusable()
    }
    fn focus_first(&mut self) -> bool {
        self.inner.focus_first()
    }
    fn set_focused(&mut self, focused: bool) {
        self.inner.set_focused(focused);
    }
    fn accepts_accelerators(&self) -> bool {
        self.inner.accepts_accelerators()
    }
    fn layout(&mut self, bounds: Rect) {
        self.bounds = bounds;
        self.win.set(Size::new(bounds.w, bounds.h));
        // Reflow the controls that track an edge: the top band follows the
        // width, the toggle row the height.
        let (w, h) = (bounds.w, bounds.h);
        self.slider.borrow_mut().set_rect(slider_rect(w));
        for (label, rect) in &self.labels {
            label.borrow_mut().rect = rect(w);
        }
        for (i, preset) in self.presets.iter().enumerate() {
            preset.borrow_mut().rect = preset_rect(i as i32, w);
        }
        self.zoom_box.borrow_mut().set_rect(zoom_rect(h));
        self.fit_box.borrow_mut().set_rect(fit_rect(h));
        self.inner.layout(bounds);
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.inner.popup_request()
    }
    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        self.inner.collect_popups(out);
    }
    fn wants_ticks(&self) -> bool {
        self.inner.wants_ticks()
    }
}

/// The preview pane. Fills the window between the controls and the toggle row
/// with a sunken canvas and renders a small panel of real widgets inside it at
/// the configured scale, so the chrome you see is drawn by the very same code
/// paths the runtime uses for the whole window — only the scale differs.
///
/// The two modes are two different `Painter` entry points, and the difference
/// between them is which of the panel's two sizes is held fixed:
///
/// * [`Painter::draw_scaled`] renders at `factor` straight onto the surface (or,
///   zoomed, magnifies the render by whole pixels), so the panel's *on-screen*
///   size grows with the factor, until the canvas runs out and clips it.
/// * [`Painter::draw_resampled`] renders at `factor` into a buffer of the
///   panel's own device pixels and resamples that into a fixed box, so the
///   on-screen size is the canvas' and the factor governs only the resolution.
struct ScalePreview {
    factor: Rc<Cell<f32>>,
    zoom: Rc<Cell<bool>>,
    fit: Rc<Cell<bool>>,
    win: Rc<Cell<Size>>,
    /// The sample widgets, positioned in preview-logical coordinates relative
    /// to the panel's top-left. They are painted, never sent events — this is a
    /// display, not an interactive surface.
    sample: Vec<Box<dyn Widget>>,
}

impl ScalePreview {
    fn new(
        factor: Rc<Cell<f32>>,
        zoom: Rc<Cell<bool>>,
        fit: Rc<Cell<bool>>,
        win: Rc<Cell<Size>>,
    ) -> Self {
        Self {
            factor,
            zoom,
            fit,
            win,
            sample: build_sample(),
        }
    }
}

/// The widgets shown inside the preview panel, laid out in preview-logical
/// coordinates within the [`SAMPLE_W`]×[`SAMPLE_H`] footprint. Two things here
/// are drawn from single logical pixels and so show a fractional scale first:
/// the scrollbar down the right-hand column — arrow glyphs and one-pixel bevels
/// — and the dotted focus rectangle inside the focused button, whose dashes are
/// every *other* pixel and alias where a scale rounds unevenly.
fn build_sample() -> Vec<Box<dyn Widget>> {
    // Parked mid-track with a thumb about a third of the track long, so both
    // the thumb and the track around it are visible at any scale.
    let mut scrollbar = ScrollBar::vertical(Rect::new(128, 24, SCROLLBAR_THICKNESS, 140));
    scrollbar.set_range(3, 6);
    scrollbar.set_value(2);

    // Focused rather than merely focusable: the panel is painted but never sent
    // events, so nothing would ever give it the focus, and the dotted rectangle
    // is the whole reason it is here.
    let mut focused = Button::new(Rect::new(10, 124, 66, 22), "Focus");
    focused.set_focused(true);

    vec![
        Box::new(Label::new(Rect::new(10, 6, 114, 14), "Preview").with_size(11.0)),
        Box::new(TextInput::new(Rect::new(10, 24, 114, 18)).with_text("Type here")),
        Box::new(
            Dropdown::new(Rect::new(10, 48, 114, 20))
                .with_items(["Apple", "Banana", "Cherry"])
                .with_selected(0),
        ),
        Box::new(Checkbox::new(Rect::new(10, 76, 114, 14), "Crisp").checked(true)),
        Box::new(Button::new(Rect::new(10, 98, 50, 22), "OK").default(true)),
        Box::new(Button::new(Rect::new(66, 98, 56, 22), "Cancel")),
        Box::new(focused),
        Box::new(ProgressBar::new(Rect::new(10, 154, 114, 10)).with_fraction(0.66)),
        Box::new(scrollbar),
    ]
}

impl Widget for ScalePreview {
    fn bounds(&self) -> Rect {
        let win = self.win.get();
        canvas_rect(win.w, win.h)
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        // Take the canvas from the *actual* window, so it tracks the live size
        // through a resize rather than the size the last layout saw.
        let win_scale = painter.scale().max(0.01);
        let logical_w = (painter.size().w as f32 / win_scale).round() as i32;
        let logical_h = (painter.size().h as f32 / win_scale).round() as i32;
        let rect = canvas_rect(logical_w, logical_h);

        // Canvas chrome: a white field with a sunken bevel, so the preview
        // reads as an inset pane rather than free-floating widgets.
        painter.fill_rect(rect, Color::WHITE);
        painter.sunken_bevel(rect, theme.highlight, theme.shadow);

        let content = rect.inset(2);
        let factor = self.factor.get().max(0.1);
        let sample = &mut self.sample;
        let draw = |p: &mut Painter| {
            for widget in sample.iter_mut() {
                widget.paint(p, theme);
            }
        };

        // Whatever the panel's footprint works out to, it is centered in the
        // canvas and clipped to it — so a window the user has dragged smaller
        // than the panel shows the middle of it rather than pushing it out of
        // the pane.
        let saved = painter.push_clip(content);
        if self.fit.get() {
            painter.draw_resampled(fit_area(content), SAMPLE, factor, Color::WHITE, draw);
        } else {
            let zoom = if self.zoom.get() { 2 } else { 1 };
            let (fw, fh) = footprint(factor, self.zoom.get(), win_scale);
            painter.draw_scaled(centered(content, fw, fh), factor, zoom, Color::WHITE, draw);
        }
        painter.restore_clip(saved);
    }
}

/// Live read-out of the configured preview scale, drawn large with a caption
/// that notes how the render is being presented.
struct FactorReadout {
    rect: Rect,
    factor: Rc<Cell<f32>>,
    zoom: Rc<Cell<bool>>,
    fit: Rc<Cell<bool>>,
}

impl FactorReadout {
    fn new(rect: Rect, factor: Rc<Cell<f32>>, zoom: Rc<Cell<bool>>, fit: Rc<Cell<bool>>) -> Self {
        Self {
            rect,
            factor,
            zoom,
            fit,
        }
    }
}

impl Widget for FactorReadout {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let factor = self.factor.get();
        if factor <= 0.0 {
            return; // OS scale not adopted yet (pre-first-paint).
        }
        painter.text(
            self.rect.x,
            self.rect.y,
            &format!("{factor:.2}x"),
            22.0,
            theme.text,
        );
        // The fit is checked first: it is the mode that overrides the zoom.
        let caption = match (self.fit.get(), self.zoom.get()) {
            (true, _) => "preview scale, fitted",
            (false, true) => "preview scale, 2x zoom",
            (false, false) => "preview scale",
        };
        painter.text(
            self.rect.x,
            self.rect.y + 24,
            caption,
            9.0,
            theme.disabled_text,
        );
    }
}

/// Bottom status bar reporting the window's *real* OS scale factor — the value
/// the display is actually set to — independent of the preview slider above. It
/// reads both scales from the painter each frame: `system_scale()` is the scale
/// the display reports (e.g. 1.50x) and `scale()` the one saudade actually
/// rasterizes at. Two things make those differ, and the bar appends the
/// rendering scale when they do: Wayland oversamples a fractional display (we
/// draw at 2.0x, the compositor resamples down to 1.5x), and everywhere else the
/// OS factor is multiplied by a correction that draws the Win 3.1 chrome a
/// quarter over its nominal 96-dpi size.
struct StatusBar {
    win: Rc<Cell<Size>>,
}

impl Widget for StatusBar {
    fn bounds(&self) -> Rect {
        // Display-only and never hit-tested; the paint path derives its real
        // position from the live window size.
        let win = self.win.get();
        Rect::new(0, win.h - FOOTER_H, win.w, FOOTER_H)
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let win_scale = painter.scale().max(0.01);
        let logical_w = (painter.size().w as f32 / win_scale).round() as i32;
        let logical_h = (painter.size().h as f32 / win_scale).round() as i32;

        // Sunken separator above the footer band, in the classic 3.1 style.
        let top = logical_h - FOOTER_H;
        painter.fill_rect(Rect::new(0, top, logical_w, 1), theme.shadow);
        painter.fill_rect(Rect::new(0, top + 1, logical_w, 1), theme.highlight);

        let system = painter.system_scale();
        let mut line = format!("System scale factor: {system:.2}x");
        // The two part company where the display's own scale isn't the one the
        // UI wants drawing at: a compositor resampling an oversampled buffer
        // down, or a base sizing a logical pixel for 90s-era glass.
        if (win_scale - system).abs() > 0.01 {
            line.push_str(&format!("    ·    Rendering at {win_scale:.2}x"));
        }
        painter.text(CANVAS_X, top + 7, &line, 10.0, theme.disabled_text);
    }
}

/// Shares a widget between the tree and the code that mutates it (the presets,
/// the OS-scale bootstrap, the layout reflow). Same adapter idea as the `timer`
/// example, generalized so one wrapper serves the slider, the buttons, the
/// labels and the checkboxes.
struct Shared<T>(Rc<RefCell<T>>);

impl<T: Widget> Widget for Shared<T> {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint(painter, theme);
    }
    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint_overlay(painter, theme);
    }
    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.0.borrow_mut().event(event, ctx);
    }
    fn captures_pointer(&self) -> bool {
        self.0.borrow().captures_pointer()
    }
    fn focusable(&self) -> bool {
        self.0.borrow().focusable()
    }
    fn focus_first(&mut self) -> bool {
        self.0.borrow_mut().focus_first()
    }
    fn set_focused(&mut self, focused: bool) {
        self.0.borrow_mut().set_focused(focused);
    }
    fn accepts_accelerators(&self) -> bool {
        self.0.borrow().accepts_accelerators()
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.0.borrow().popup_request()
    }
    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        self.0.borrow().collect_popups(out);
    }
    fn wants_ticks(&self) -> bool {
        self.0.borrow().wants_ticks()
    }
}
