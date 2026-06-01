//! timer — 7GUIs task 4.
//!
//! "A gauge G for the elapsed time e, a label showing e as a number, a slider
//! S to adjust the duration d while the timer runs, and a reset button R.
//! Adjusting S takes effect immediately. When e ≥ d the timer stops (G full);
//! raising d above e starts it ticking again. R resets e to zero."
//!
//! Pieces:
//! * G  → [`ProgressBar`] (the gauge), filled to `e / d`.
//! * S  → [`Slider`] over `0..=30` seconds; its `on_change` re-fills G live.
//! * R  → [`Button`] that zeroes `e`.
//! * the clock → an invisible `TimerEngine` widget that takes `Event::Tick`
//!   and advances `e` by real wall-clock time while `e < d`.
//!
//! Everything that more than one of those needs to touch (`e`, the gauge, the
//! elapsed label) lives behind a shared cell.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use saudade::{
    App, Button, Container, Event, EventCtx, Label, Painter, ProgressBar, Rect, Slider, Theme,
    Widget, WindowConfig,
};

const W: i32 = 300;
const H: i32 = 176;
/// Longest duration the slider can dial in, in seconds.
const MAX_SECONDS: i32 = 30;
const DEFAULT_SECONDS: i32 = 15;

fn main() {
    // Elapsed time in seconds. Shared between the clock engine (writes it),
    // the slider's live re-fill, and the reset button (zeroes it).
    let elapsed = Rc::new(RefCell::new(0.0f64));
    let gauge = Rc::new(RefCell::new(ProgressBar::new(Rect::new(96, 16, 188, 16))));
    let elapsed_label = Rc::new(RefCell::new(Label::new(
        Rect::new(96, 40, 188, 16),
        "0.0 s",
    )));
    let duration_label = Rc::new(RefCell::new(Label::new(
        Rect::new(96, 96, 188, 16),
        format!("{DEFAULT_SECONDS} s"),
    )));

    let slider = Slider::new(Rect::new(96, 64, 188, 24), 0, MAX_SECONDS)
        .with_value(DEFAULT_SECONDS)
        .on_change({
            let elapsed = elapsed.clone();
            let gauge = gauge.clone();
            let duration_label = duration_label.clone();
            move |cx, d| {
                // The gauge must follow the duration *immediately*, even while
                // the timer is stopped — so recompute the fill right here
                // instead of waiting for the next tick.
                let e = *elapsed.borrow();
                let frac = if d > 0 { (e / d as f64).min(1.0) } else { 1.0 };
                gauge.borrow_mut().set_fraction(frac as f32);
                duration_label.borrow_mut().text = format!("{d} s");
                cx.request_paint();
            }
        });
    let slider = Rc::new(RefCell::new(slider));

    let reset = Button::new(Rect::new(110, 128, 80, 24), "Reset").on_click({
        let elapsed = elapsed.clone();
        let gauge = gauge.clone();
        let elapsed_label = elapsed_label.clone();
        move |cx| {
            *elapsed.borrow_mut() = 0.0;
            gauge.borrow_mut().set_fraction(0.0);
            elapsed_label.borrow_mut().text = "0.0 s".to_string();
            cx.request_paint();
        }
    });

    let engine = TimerEngine {
        elapsed: elapsed.clone(),
        gauge: gauge.clone(),
        elapsed_label: elapsed_label.clone(),
        slider: slider.clone(),
        last: None,
    };

    let root = Container::new(W, H)
        .add(Label::new(Rect::new(16, 16, 76, 16), "Elapsed:"))
        .add(SharedProgressBar(gauge.clone()))
        .add(SharedLabel(elapsed_label.clone()))
        .add(Label::new(Rect::new(16, 68, 76, 16), "Duration:"))
        .add(SharedSlider(slider.clone()))
        .add(SharedLabel(duration_label.clone()))
        .add(reset)
        .add(engine);

    App::new(WindowConfig::new("Timer", W, H), root)
        .with_theme(Theme::windows_31())
        .run();
}

/// Invisible widget that turns `Event::Tick` into elapsed-time progress. It
/// owns nothing visible — zero bounds keep it out of hit-testing — and exists
/// purely to read the duration off the slider, accumulate wall-clock time into
/// `elapsed`, and push the result to the gauge and the elapsed label.
struct TimerEngine {
    elapsed: Rc<RefCell<f64>>,
    gauge: Rc<RefCell<ProgressBar>>,
    elapsed_label: Rc<RefCell<Label>>,
    slider: Rc<RefCell<Slider>>,
    /// Instant of the previous accumulating tick. Cleared whenever the timer
    /// isn't running so resuming (after raising the duration) never folds the
    /// idle gap into `elapsed`.
    last: Option<Instant>,
}

impl Widget for TimerEngine {
    fn bounds(&self) -> Rect {
        Rect::new(0, 0, 0, 0)
    }

    fn paint(&mut self, _painter: &mut Painter, _theme: &Theme) {}

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if !matches!(event, Event::Tick) {
            return;
        }
        let now = Instant::now();
        let d = self.slider.borrow().value() as f64;

        let mut e = self.elapsed.borrow_mut();
        if *e < d {
            if let Some(prev) = self.last {
                *e = (*e + now.duration_since(prev).as_secs_f64()).min(d);
            }
            // Keep ticking only while there's still time left; otherwise drop
            // `last` so a later resume doesn't count the paused interval.
            self.last = if *e < d { Some(now) } else { None };
        } else {
            self.last = None;
        }
        let elapsed = *e;
        drop(e);

        let frac = if d > 0.0 { (elapsed / d).min(1.0) } else { 1.0 };
        self.gauge.borrow_mut().set_fraction(frac as f32);
        self.elapsed_label.borrow_mut().text = format!("{elapsed:.1} s");
        ctx.request_paint();
    }

    fn wants_ticks(&self) -> bool {
        *self.elapsed.borrow() < self.slider.borrow().value() as f64
    }
}

// ============================================================================
// Shared adapters — the same pattern as the picker/notepad examples, letting
// callbacks and the widget tree both hold the live widget.
// ============================================================================

struct SharedProgressBar(Rc<RefCell<ProgressBar>>);

impl Widget for SharedProgressBar {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint(painter, theme);
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
}

struct SharedLabel(Rc<RefCell<Label>>);

impl Widget for SharedLabel {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint(painter, theme);
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
}

struct SharedSlider(Rc<RefCell<Slider>>);

impl Widget for SharedSlider {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint(painter, theme);
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
    fn set_focused(&mut self, focused: bool) {
        self.0.borrow_mut().set_focused(focused);
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
}
