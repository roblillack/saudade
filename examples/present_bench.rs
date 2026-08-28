//! EXPERIMENTAL, uncommitted: a repaint treadmill for comparing the two
//! presenters behind `src/present.rs`.
//!
//! It fills a window with the kind of work a real frame does — a patterned
//! backdrop, bevelled chrome, a few hundred glyphs, a scaled image — repaints
//! it on every tick, and (with `SAUDADE_FRAME_STATS` set) prints how long the
//! painting and the handing-over took. A menu is left hanging open off the
//! window's bottom edge so popup frames, and the cost of building a popup
//! window's surface, land in the numbers too.
//!
//! ```sh
//! # the CPU path (softbuffer)
//! SAUDADE_FRAME_STATS=120 cargo run --release --example present_bench
//! # the GPU path through `pixels`
//! SAUDADE_FRAME_STATS=120 cargo run --release --features pixels-backend \
//!     --example present_bench
//! # the GPU path driven directly, one device for every window
//! SAUDADE_FRAME_STATS=120 cargo run --release --features wgpu-backend \
//!     --example present_bench
//! ```
//!
//! The wgpu backend also takes `SAUDADE_WGPU_MODE=quad` (draw the frame with a
//! fullscreen triangle instead of writing it into the swapchain image),
//! `SAUDADE_WGPU_PAD=0` (tight rows rather than 256-byte-aligned ones) and
//! `SAUDADE_GPU_VSYNC=1`.
//!
//! `BENCH_SIZE=1200x800` sets the window size, `BENCH_FRAMES=600` how many
//! frames to run before quitting, and `BENCH_POPUP` picks what the popups do:
//! `open` (the default) leaves a menu hanging open, `0` leaves it shut, and
//! `cycle` opens and closes a context menu every 30 frames, so the cost of
//! *building* a popup window's surface is sampled over and over — that is where
//! a backend that wants a GPU device per window shows itself.

use std::cell::RefCell;
use std::env;
use std::rc::Rc;

use saudade::{
    App, Button, Checkbox, Color, Container, ContextMenu, Event, EventCtx, Label, Menu, MenuBar,
    MenuItem, Painter, Point, PopupRequest, ProgressBar, Rect, Slider, Theme, Widget, WindowConfig,
};

const BAR_H: i32 = 20;

fn main() {
    let (w, h) = env::var("BENCH_SIZE")
        .ok()
        .and_then(|s| {
            let (a, b) = s.split_once('x')?;
            Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
        })
        .unwrap_or((900, 600));
    let frames: u32 = env::var("BENCH_FRAMES")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(600);
    let popup = env::var("BENCH_POPUP").unwrap_or_else(|_| "open".into());
    let (menu_open, cycle) = match popup.as_str() {
        "0" | "none" | "closed" => (false, false),
        "cycle" => (false, true),
        _ => (true, false),
    };

    println!("present_bench: {w}x{h} logical, {frames} frames, popup {popup}");
    println!("set SAUDADE_FRAME_STATS=120 for the timings");

    let mut bar = MenuBar::new(Rect::new(0, 0, w, BAR_H)).add_menu(Menu::new(
        "&Bench",
        vec![
            MenuItem::action("&Repaint", |_cx| {}),
            MenuItem::action("&Measure", |_cx| {}),
            MenuItem::separator(),
            MenuItem::action("&Quit", |cx| cx.close()),
        ],
    ));
    if menu_open {
        bar.open(0);
    }

    let mut root = Container::new(w, h).with_background(Color::LIGHT_GRAY);
    root.push(bar);

    // Chrome down the left: bevels, glyphs, a slider and a bar are the shapes
    // the painter actually spends its time on.
    let mut y = BAR_H + 16;
    for i in 0..10 {
        root.push(Label::new(
            Rect::new(16, y, 340, 16),
            format!("Row {i}: the quick brown fox jumps over the lazy dog"),
        ));
        y += 18;
        root.push(Button::new(
            Rect::new(16, y, 160, 24),
            format!("Button {i}"),
        ));
        y += 30;
    }
    root.push(Checkbox::new(Rect::new(16, y, 200, 18), "A checkbox"));
    y += 24;
    root.push(Slider::new(Rect::new(16, y, 240, 20), 0, 100).with_value(42));
    y += 26;
    root.push(ProgressBar::new(Rect::new(16, y, 240, 18)).with_fraction(0.42));

    // Paragraph text on the right, so a good share of the window is glyphs.
    let mut ty = BAR_H + 16;
    for i in 0..24 {
        root.push(Label::new(
            Rect::new(400, ty, w - 420, 14),
            format!("{i:02}  Sphinx of black quartz, judge my vow — 0123456789 — {i:02}"),
        ));
        ty += 16;
    }

    // The churn target: a context menu the treadmill opens and closes, so a
    // popup window (and its surface) is built over and over.
    let churn = Rc::new(RefCell::new(ContextMenu::new().with_items(vec![
        MenuItem::action("Cut", |_cx| {}),
        MenuItem::action("Copy", |_cx| {}),
        MenuItem::action("Paste", |_cx| {}),
    ])));
    root.push(SharedMenu(churn.clone()));

    root.push(Treadmill {
        left: Rc::new(RefCell::new(frames)),
        tick: 0,
        churn: cycle.then_some(churn),
        anchor: Point::new(w / 2, h / 2),
    });

    App::new(WindowConfig::new("present_bench", w, h), root)
        .with_theme(Theme::windows_31())
        .run();
}

/// Asks for a repaint on every tick, and closes the window once it has had
/// `left` of them. Draws nothing itself.
struct Treadmill {
    left: Rc<RefCell<u32>>,
    tick: u32,
    /// `Some` in cycle mode: opened and closed every 30 frames.
    churn: Option<Rc<RefCell<ContextMenu>>>,
    anchor: Point,
}

impl Widget for Treadmill {
    fn bounds(&self) -> Rect {
        Rect::new(0, 0, 0, 0)
    }
    fn paint(&mut self, _painter: &mut Painter, _theme: &Theme) {}
    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if !matches!(event, Event::Tick) {
            return;
        }
        self.tick += 1;
        if let Some(menu) = self.churn.as_ref() {
            // 30 frames open, 30 shut. Each open builds a fresh popup window.
            let mut menu = menu.borrow_mut();
            if self.tick.is_multiple_of(30) {
                if menu.is_open() {
                    menu.close();
                } else {
                    menu.open_at(self.anchor);
                }
            }
        }
        let mut left = self.left.borrow_mut();
        *left = left.saturating_sub(1);
        if *left == 0 {
            ctx.close();
        } else {
            ctx.request_paint();
        }
    }
    fn wants_ticks(&self) -> bool {
        true
    }
    fn layout(&mut self, _bounds: Rect) {}
    fn popup_request(&self) -> Option<PopupRequest> {
        None
    }
}

/// Shares the churned context menu between the treadmill (which opens it) and
/// the widget tree (which draws it and asks for its popup window).
struct SharedMenu(Rc<RefCell<ContextMenu>>);

impl Widget for SharedMenu {
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
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.0.borrow().popup_request()
    }
    fn collect_popups(&self, out: &mut Vec<PopupRequest>) {
        self.0.borrow().collect_popups(out);
    }
}
