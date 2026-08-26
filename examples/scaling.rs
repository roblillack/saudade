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
//! bar, a scrollbar — at that scale, in a canvas below the controls. The slider starts at this display's actual OS scale, so the panel
//! opens looking exactly like the rest of the window.
//!
//! The window resizes to the *space the preview needs*: whenever the scale
//! factor (or the 2× zoom) changes, the example recomputes how big the panel
//! will render and asks the runtime to resize the window — via
//! [`EventCtx::request_window_size`] — so the panel always fits, with the
//! controls and canvas reflowing to the new width. Drag the slider up and the
//! window grows; drag it down and the window shrinks.
//!
//! The factor is the *absolute* logical→physical scale, the same number the OS
//! reports. Try the fractional steps (1.25x, 1.5x): that's where saudade's
//! crisp physical-pixel chrome pass earns its keep. The presets walk the
//! quarter ladder a macOS density correction snaps to — the only scales the
//! runtime itself ever picks — up into the range where a logical pixel is worth
//! two-and-a-fraction physical ones and every edge has to round. The "Zoom in 2x" checkbox
//! magnifies the *rendered result* 2× (a pure pixel copy — it does not re-run
//! the scaling at a higher factor) so you can see the per-pixel snapping a scale
//! produced.
//!
//! A status bar along the bottom reports the window's *actual* OS scale factor
//! (`Painter::system_scale`) — independent of the preview slider above — and,
//! when it differs, the scale the window is really being drawn at. It differs
//! on Wayland, where a fractional display (say 150%) is oversampled at 2.0x and
//! resampled down by the compositor, and on macOS, where saudade multiplies the
//! backing factor by a correction for the display's physical density.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use saudade::{
    App, Button, Checkbox, Color, Container, Dropdown, Event, EventCtx, Label, Painter,
    PopupRequest, ProgressBar, Rect, SCROLLBAR_THICKNESS, ScrollBar, Slider, TextInput, Theme,
    Widget, WindowConfig,
};

/// Layout metrics. The controls occupy a fixed-height band at the top (down to
/// `CANVAS_Y`) and need at least `MIN_W` to lay out; below them the preview
/// canvas is grown to fit the sample panel at the current scale, with
/// `PANEL_PAD` of breathing room, and the window is sized to match. So changing
/// the scale factor (or the 2× zoom) resizes the window to the needed space.
const MIN_W: i32 = 480;
const CANVAS_X: i32 = 24;
const MARGIN: i32 = 16;
const PANEL_PAD: i32 = 24;
/// Height of the bottom status bar that reports the real OS scale factor.
const FOOTER_H: i32 = 24;

/// The preset grid: `PRESET_ROWS` rows of `PRESET_COLS` buttons, starting at
/// `PRESET_Y`. The rows below derive their own y from these, so adding a row of
/// presets moves the zoom toggle and the canvas down with it.
const PRESET_W: i32 = 80;
const PRESET_H: i32 = 24;
const PRESET_COLS: i32 = 5;
const PRESET_ROWS: i32 = 2;
const PRESET_Y: i32 = 138;
const PRESET_ROW_GAP: i32 = 4;
/// First free row under the preset grid.
const PRESETS_BOTTOM: i32 = PRESET_Y + PRESET_ROWS * PRESET_H + (PRESET_ROWS - 1) * PRESET_ROW_GAP;
/// The "Zoom in 2x" checkbox, and the canvas that starts below it.
const ZOOM_Y: i32 = PRESETS_BOTTOM + 10;
const ZOOM_H: i32 = 16;
const CANVAS_Y: i32 = ZOOM_Y + ZOOM_H + 10;

/// Slider range, in percent (100% = 1.0x … 350% = 3.5x). The top reaches the
/// densest preset rather than a round number, so every preset is a position the
/// slider can also be dragged to.
const MIN_PCT: i32 = 100;
const MAX_PCT: i32 = 350;
/// Preset scales, ascending, filling the grid row by row: the first row runs
/// 1.0x to 2.0x and the second carries on to 3.0x, with a 3.5x at the end.
///
/// Every one is a quarter step, because a quarter is all a macOS density
/// correction ever snaps to — so the grid is the ladder itself, walked in
/// order. It spans what both kinds of Mac can land on: the low steps are a
/// dense display on a 1x machine, the 2.25x–2.75x middle is where a Retina Mac
/// ends up (a 27" 4K measures 2.25x, a 14" MacBook Pro panel 2.75x), and 3.5x
/// is the far end.
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

/// The sample panel's on-screen footprint (after the optional 2× zoom) in the
/// window's logical pixels, given the OS scale `os_scale`. This is the literal
/// space `draw_scaled` will fill: the panel is rendered at `factor` and then
/// magnified by `zoom`, so its physical size is `SAMPLE × factor × zoom`, which
/// divided by `os_scale` gives logical pixels.
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
fn window_for_footprint(fw: i32, fh: i32) -> (i32, i32) {
    let w = (fw + 2 * (PANEL_PAD + CANVAS_X)).max(MIN_W);
    let h = CANVAS_Y + fh + 2 * PANEL_PAD + MARGIN + FOOTER_H;
    (w, h)
}

/// Window size for the given scale + zoom at OS scale `os_scale`.
fn desired_window(factor: f32, zoom: bool, os_scale: f32) -> (i32, i32) {
    let (fw, fh) = footprint(factor, zoom, os_scale);
    window_for_footprint(fw, fh)
}

/// The slider spans from a fixed left edge out to the right margin of a window
/// `w` wide.
fn slider_rect(w: i32) -> Rect {
    Rect::new(140, 92, w - 164, 22)
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

fn main() {
    // Shared state. `factor` is the scale the preview renders at — 0.0 until the
    // first paint adopts the OS scale (see `Root::paint`). `zoom` is the 2×
    // magnify toggle. `os_scale` caches the OS scale, refreshed every paint, so
    // the event handlers can size the window without a painter in hand.
    let factor = Rc::new(Cell::new(0.0_f32));
    let zoom = Rc::new(Cell::new(false));
    let os_scale = Rc::new(Cell::new(1.0_f32));

    // The window opens at the default scale (factor == OS scale), where the
    // panel renders at its natural `SAMPLE` size regardless of the display.
    let (init_w, init_h) = window_for_footprint(SAMPLE_W, SAMPLE_H);

    // Resize the window to fit the panel at the new scale + zoom.
    let resize = {
        let zoom = zoom.clone();
        let os_scale = os_scale.clone();
        move |cx: &mut EventCtx, factor: f32| {
            let (w, h) = desired_window(factor, zoom.get(), os_scale.get());
            cx.request_window_size(w, h);
        }
    };

    // The width-spanning controls are shared so the root can reposition them in
    // `layout` when a resize changes the width — the same instances the
    // container routes events and paints through.
    let slider = Rc::new(RefCell::new(
        Slider::new(slider_rect(init_w), MIN_PCT, MAX_PCT)
            .with_step(5)
            .on_change({
                let factor = factor.clone();
                let resize = resize.clone();
                move |cx, pct| {
                    let f = pct as f32 / 100.0;
                    factor.set(f);
                    resize(cx, f);
                }
            }),
    ));
    let max_tick = Rc::new(RefCell::new(
        Label::new(max_tick_rect(init_w), "3.5x")
            .with_size(9.0)
            .with_color(Color::DARK_GRAY),
    ));
    let presets: Vec<Rc<RefCell<Button>>> = PRESETS
        .iter()
        .enumerate()
        .map(|(i, &(label, pct))| {
            let button = Button::new(preset_rect(i as i32, init_w), label).on_click({
                let slider = slider.clone();
                let factor = factor.clone();
                let resize = resize.clone();
                move |cx| {
                    let f = pct as f32 / 100.0;
                    slider.borrow_mut().set_value(pct);
                    factor.set(f);
                    resize(cx, f);
                }
            });
            Rc::new(RefCell::new(button))
        })
        .collect();

    // Toggling zoom also resizes the window — the panel doubles, so the space it
    // needs doubles too.
    let zoom_toggle = Checkbox::new(Rect::new(24, ZOOM_Y, init_w - 48, ZOOM_H), "Zoom in 2x")
        .on_toggle({
            let zoom = zoom.clone();
            let factor = factor.clone();
            let os_scale = os_scale.clone();
            move |cx, on| {
                zoom.set(on);
                let (w, h) = desired_window(factor.get().max(0.1), on, os_scale.get());
                cx.request_window_size(w, h);
            }
        });

    let mut body = Container::new(init_w, init_h)
        .add(Label::new(Rect::new(24, 16, init_w - 48, 18), "Scale factor preview").with_size(13.0))
        .add(
            Label::new(
                Rect::new(24, 38, init_w - 48, 42),
                "Render a panel of widgets at any logical-to-physical scale — the\n\
                 window's own scale never changes, but it resizes to fit the\n\
                 preview. Zoom magnifies the rendered result 2x to reveal pixels.",
            )
            .with_size(10.0),
        )
        .add(FactorReadout::new(
            Rect::new(24, 88, 110, 34),
            factor.clone(),
            zoom.clone(),
        ))
        .add(Label::new(Rect::new(140, 118, 40, 14), "1.0x").with_size(9.0))
        .add(Shared(max_tick.clone()))
        .add(Shared(slider.clone()));
    for preset in &presets {
        body.push(Shared(preset.clone()));
    }
    body.push(zoom_toggle);
    body.push(ScalePreview::new(
        factor.clone(),
        zoom.clone(),
        os_scale.clone(),
    ));
    body.push(StatusBar);

    App::new(
        WindowConfig::new("Scale Factor", init_w, init_h),
        Root::new(body, factor.clone(), os_scale, slider, max_tick, presets),
    )
    .with_theme(Theme::windows_31())
    .run();
}

/// Root wrapper. It lets the content fill the window instead of being centered
/// at a fixed design size (it reports its allocated bounds, so the runtime never
/// letterboxes, and floods them white), reflows the width-spanning controls
/// when the window resizes, refreshes the cached OS scale, and owns the
/// first-paint bootstrap — then defers everything else to the inner
/// [`Container`].
struct Root {
    inner: Container,
    bounds: Rect,
    factor: Rc<Cell<f32>>,
    os_scale: Rc<Cell<f32>>,
    // The width-spanning controls, repositioned on every `layout`.
    slider: Rc<RefCell<Slider>>,
    max_tick: Rc<RefCell<Label>>,
    presets: Vec<Rc<RefCell<Button>>>,
}

impl Root {
    fn new(
        inner: Container,
        factor: Rc<Cell<f32>>,
        os_scale: Rc<Cell<f32>>,
        slider: Rc<RefCell<Slider>>,
        max_tick: Rc<RefCell<Label>>,
        presets: Vec<Rc<RefCell<Button>>>,
    ) -> Self {
        let (w, h) = window_for_footprint(SAMPLE_W, SAMPLE_H);
        Self {
            inner,
            bounds: Rect::new(0, 0, w, h),
            factor,
            os_scale,
            slider,
            max_tick,
            presets,
        }
    }
}

impl Widget for Root {
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        // Keep the cached OS scale fresh for the resize math in the handlers.
        let os = painter.scale();
        self.os_scale.set(os);
        // First paint: adopt the OS scale as the starting preview scale and move
        // the slider thumb to match, before any child reads either.
        if self.factor.get() <= 0.0 {
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
        // Reflow the width-spanning controls to the window's current width.
        let w = bounds.w;
        self.slider.borrow_mut().set_rect(slider_rect(w));
        self.max_tick.borrow_mut().rect = max_tick_rect(w);
        for (i, preset) in self.presets.iter().enumerate() {
            preset.borrow_mut().rect = preset_rect(i as i32, w);
        }
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

/// The preview pane. Fills the window below the controls with a sunken canvas
/// and renders a small panel of real widgets inside it at the configured scale
/// via [`Painter::draw_scaled`], so the chrome you see is drawn by the very same
/// code paths the runtime uses for the whole window — only the scale (and the
/// optional 2× zoom) differ.
struct ScalePreview {
    factor: Rc<Cell<f32>>,
    zoom: Rc<Cell<bool>>,
    os_scale: Rc<Cell<f32>>,
    /// The sample widgets, positioned in preview-logical coordinates relative
    /// to the panel's top-left. They are painted, never sent events — this is a
    /// display, not an interactive surface.
    sample: Vec<Box<dyn Widget>>,
}

impl ScalePreview {
    fn new(factor: Rc<Cell<f32>>, zoom: Rc<Cell<bool>>, os_scale: Rc<Cell<f32>>) -> Self {
        Self {
            factor,
            zoom,
            os_scale,
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
        // Mirror what `window_for_footprint` reserves for the canvas — not
        // hit-test-critical (the preview is inert), just honest.
        let (w, h) = desired_window(
            self.factor.get().max(0.1),
            self.zoom.get(),
            self.os_scale.get(),
        );
        Rect::new(
            CANVAS_X,
            CANVAS_Y,
            w - 2 * CANVAS_X,
            (h - CANVAS_Y - MARGIN - FOOTER_H).max(40),
        )
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let win_scale = painter.scale().max(0.01);
        // Fill the canvas to the edges of the *actual* window, so it tracks the
        // live size while a resize settles rather than the target.
        let logical_w = (painter.size().w as f32 / win_scale).round() as i32;
        let logical_h = (painter.size().h as f32 / win_scale).round() as i32;
        let rect = Rect::new(
            CANVAS_X,
            CANVAS_Y,
            (logical_w - 2 * CANVAS_X).max(40),
            (logical_h - CANVAS_Y - MARGIN - FOOTER_H).max(40),
        );

        // Canvas chrome: a white field with a sunken bevel, so the preview
        // reads as an inset pane rather than free-floating widgets.
        painter.fill_rect(rect, Color::WHITE);
        painter.sunken_bevel(rect, theme.highlight, theme.shadow);

        let factor = self.factor.get().max(0.1);
        let zoom = if self.zoom.get() { 2 } else { 1 };

        // The panel's on-screen footprint (after zoom) in this window's logical
        // pixels, centered in the canvas. The window is sized to fit it, so it
        // sits with `PANEL_PAD` of breathing room; if a resize is still settling
        // and the canvas is briefly too small, the clip below trims the overflow.
        let content = rect.inset(2);
        let fw = (SAMPLE_W as f32 * factor * zoom as f32 / win_scale)
            .round()
            .max(1.0) as i32;
        let fh = (SAMPLE_H as f32 * factor * zoom as f32 / win_scale)
            .round()
            .max(1.0) as i32;
        let area = Rect::new(
            content.x + (content.w - fw) / 2,
            content.y + (content.h - fh) / 2,
            fw,
            fh,
        );

        let saved = painter.push_clip(content);
        painter.draw_scaled(area, factor, zoom, Color::WHITE, |p| {
            for widget in &mut self.sample {
                widget.paint(p, theme);
            }
        });
        painter.restore_clip(saved);
    }
}

/// Live read-out of the configured preview scale, drawn large with a caption
/// that notes when the 2× zoom is on.
struct FactorReadout {
    rect: Rect,
    factor: Rc<Cell<f32>>,
    zoom: Rc<Cell<bool>>,
}

impl FactorReadout {
    fn new(rect: Rect, factor: Rc<Cell<f32>>, zoom: Rc<Cell<bool>>) -> Self {
        Self { rect, factor, zoom }
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
        let caption = if self.zoom.get() {
            "preview scale, 2x zoom"
        } else {
            "preview scale"
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
/// rasterizes at. Two backends make those differ, and the bar appends the
/// rendering scale when they do: Wayland oversamples a fractional display (we
/// draw at 2.0x, the compositor resamples down to 1.5x), and macOS multiplies
/// in a density correction, since a Mac's scale factor says only whether the
/// panel is Retina and nothing about how dense it is.
struct StatusBar;

impl Widget for StatusBar {
    fn bounds(&self) -> Rect {
        // Display-only and never hit-tested; the paint path derives its real
        // position from the live window size. A nominal footer-row rect.
        Rect::new(0, 0, MIN_W, FOOTER_H)
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
        // down, or a density correction sizing a logical pixel for the glass.
        if (win_scale - system).abs() > 0.01 {
            line.push_str(&format!("    ·    Rendering at {win_scale:.2}x"));
        }
        painter.text(CANVAS_X, top + 7, &line, 10.0, theme.disabled_text);
    }
}

/// Shares a widget between the tree and the code that mutates it (the presets,
/// the OS-scale bootstrap, the resize reflow). Same adapter idea as the `timer`
/// example, generalized so one wrapper serves the slider, the buttons, and the
/// tick label.
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
