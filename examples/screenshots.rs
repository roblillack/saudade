//! screenshots — renders the README's pictures: the headline shot at the top,
//! and one of every example.
//!
//! Each example is pulled in below as a module, so a screenshot shows the very
//! window `cargo run --example <name>` opens: the example's own `window()`
//! builds the widget tree, saudade's offscreen [`MockBackend`] lays it out and
//! paints it, a few synthetic events stage whatever is worth showing (a typed
//! value, a drawn circle, an open menu), and [`MockBackend::render_framed`]
//! wraps the result in Canoe-style chrome — title bar, frame and drop shadow on
//! the teal desktop. The headline shot arranges several windows on one
//! [`Desktop`] instead: the picker, with the dialog its OK button opens in
//! front, and the timer and the temperature converter beside it. No window (or
//! windowing system) is involved.
//!
//! The shots are captured at 2× so they stay crisp on hi-DPI displays, and set
//! in the host's own fonts, like the examples themselves — so they come out
//! looking the way the examples do on the machine that renders them. Run it to
//! refresh the images under `docs/`:
//!
//! ```sh
//! cargo run --example screenshots
//! ```

// A program, not a test: the examples it includes carry unit tests of their
// own, which their own test targets already run.
#![cfg(not(test))]

use std::path::{Path, PathBuf};
use std::time::Duration;

use saudade::mock::{Desktop, MockBackend, Snapshot};
use saudade::{
    DragData, Event, Font, Key, Modifiers, MouseButton, NamedKey, Point, PopupKind, Size, Widget,
    WindowChrome, WindowConfig,
};

// Every example, as a module. Only its `window()` is called from here, which
// leaves the rest of what its own binary needs (`main`, mostly) unused.
#[allow(dead_code)]
#[path = "cells.rs"]
mod cells;
#[allow(dead_code)]
#[path = "chrome.rs"]
mod chrome;
#[allow(dead_code)]
#[path = "circle_drawer.rs"]
mod circle_drawer;
#[allow(dead_code)]
#[path = "counter.rs"]
mod counter;
#[allow(dead_code)]
#[path = "credit_card.rs"]
mod credit_card;
#[allow(dead_code)]
#[path = "crud.rs"]
mod crud;
#[allow(dead_code)]
#[path = "dialogs.rs"]
mod dialogs;
#[allow(dead_code)]
#[path = "dnd.rs"]
mod dnd;
#[allow(dead_code)]
#[path = "filer.rs"]
mod filer;
#[allow(dead_code)]
#[path = "flight_booker.rs"]
mod flight_booker;
#[allow(dead_code)]
#[path = "focus_form.rs"]
mod focus_form;
#[allow(dead_code)]
#[path = "fonts.rs"]
mod fonts;
#[allow(dead_code)]
#[path = "fullscreen.rs"]
mod fullscreen;
#[allow(dead_code)]
#[path = "notepad.rs"]
mod notepad;
#[allow(dead_code)]
#[path = "patterns.rs"]
mod patterns;
#[allow(dead_code)]
#[path = "picker.rs"]
mod picker;
#[allow(dead_code)]
#[path = "scaling.rs"]
mod scaling;
#[allow(dead_code)]
#[path = "svg.rs"]
mod svg;
#[allow(dead_code)]
#[path = "temperature.rs"]
mod temperature;
#[allow(dead_code)]
#[path = "timer.rs"]
mod timer;

/// Capture the windows at 2× so the README images stay crisp on hi-DPI
/// displays.
const SCALE: f32 = 2.0;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let docs = root.join("docs");
    let out = docs.join("screenshots");
    std::fs::create_dir_all(&out).expect("create docs/screenshots/");

    headline(&docs.join("screenshot.png"));

    // Positions below are logical pixels in the example's window, as laid out
    // by its own code.
    shoot(
        &out,
        "notepad",
        notepad::window(root.join("LICENSE")),
        |w| {
            w.click(14, 10); // the File menu
        },
    );
    shoot(&out, "filer", filer::window(root.join("examples")), |_| {});
    shoot(&out, "dnd", dnd::window(), |w| {
        w.drop_files(&[
            "/home/ada/Documents/notes.txt",
            "/home/ada/Pictures/lisbon.jpg",
            "/home/ada/Music/fado.flac",
        ]);
    });
    shoot(&out, "fullscreen", fullscreen::window(false), |_| {});
    shoot(&out, "picker", picker::window(), |w| {
        w.ctrl_click(60, 76); // Capers
        w.ctrl_click(60, 124); // Fennel
    });
    shoot(&out, "focus_form", focus_form::window(), |w| {
        w.type_text("Lovelace");
        w.key(NamedKey::Tab);
        w.type_text("12 St James's Square");
        w.key(NamedKey::Tab);
        w.type_text("ada@example.com");
    });
    shoot(&out, "counter", counter::window(), |w| {
        for _ in 0..3 {
            w.click(162, 32); // Count
        }
    });
    shoot(
        &out,
        "temperature",
        temperature::window(),
        stage_temperature,
    );
    shoot(&out, "flight_booker", flight_booker::window(), |w| {
        w.click(150, 28); // open the flight-type dropdown
    });
    shoot(&out, "timer", timer::window(), stage_timer);
    shoot(&out, "crud", crud::window(), |w| {
        w.click(80, 74); // Mustermann, Max
    });
    shoot(&out, "circle_drawer", circle_drawer::window(), |w| {
        for (x, y) in [
            (90, 130),
            (170, 220),
            (260, 150),
            (340, 280),
            (130, 320),
            (400, 110),
        ] {
            w.click(x, y);
        }
        w.right_click(260, 150); // select a circle and open its menu
    });
    shoot(&out, "cells", cells::window(), |_| {});
    shoot(&out, "patterns", patterns::window(), |w| {
        w.char_key('p'); // diagonal → cross-stitch
        w.char_key('c'); // superlight → light
    });
    shoot(&out, "fonts", fonts::window(), |_| {});
    shoot(&out, "scaling", scaling::window(), |_| {});
    shoot(&out, "credit_card", credit_card::window(), |_| {});
    shoot(&out, "svg", svg::window(), |_| {});
    shoot(&out, "dialogs", dialogs::window(), |_| {});

    // The chrome example frames a bare about box rather than opening a window
    // of its own; picture it the way it pictures itself.
    let about = chrome::about_box();
    let size = about.bounds();
    let config = WindowConfig::new("About Saudade", size.w, size.h);
    shoot(&out, "chrome", (config, about), |_| {});
}

fn stage_temperature(w: &mut Driver) {
    w.type_text("100");
}

fn stage_timer(w: &mut Driver) {
    // The slider has focus: three pages down shortens the duration from 15 s
    // to 6 s. Then let the clock run for a while.
    for _ in 0..3 {
        w.key(NamedKey::PageDown);
    }
    w.tick();
    std::thread::sleep(Duration::from_millis(2400));
    w.tick();
}

/// The README's headline shot, into `path`: the picker, its OK just pressed,
/// with the dialog that opens in front; to its right, the timer and the
/// temperature converter. Only the dialog is active, as it has the focus.
fn headline(path: &Path) {
    let mut picker = Shot::open(picker::window());
    picker.stage(|w| w.click(136, 248)); // OK
    let mut timer = Shot::open(timer::window());
    timer.stage(stage_timer);
    let mut temperature = Shot::open(temperature::window());
    temperature.stage(stage_temperature);

    // The picker on the left, the other two stacked on its right, `GAP`
    // logical pixels apart and from the desktop's edges.
    const GAP: i32 = 24;
    let (p, c, t) = (picker.frame(), temperature.frame(), timer.frame());
    let right = GAP + p.w + GAP;
    let mut desktop = Desktop::new(
        right + c.w.max(t.w) + GAP,
        (GAP + p.h).max(GAP + c.h + GAP + t.h) + GAP,
        SCALE,
    );
    picker.place_inactive(&mut desktop, Point::new(GAP, GAP));
    temperature.place_inactive(&mut desktop, Point::new(right, GAP));
    timer.place_inactive(&mut desktop, Point::new(right, GAP + c.h + GAP));

    // The dialog in front, from just under the picker's Fennel row: three
    // quarters of it over the picker, a quarter over the timer, where it hides
    // only labels.
    let dialog = picker.dialog_frame().expect("OK opens a dialog");
    let at = Point::new(right - dialog.w * 3 / 4, 174);
    picker
        .backend
        .place_dialog(&mut desktop, at, &mut picker.root);

    write_png(path, desktop.snapshot());
}

/// Render an example's window into `out/<name>.png`, framed in the chrome a
/// window like it gets, once `stage` has driven it into the state worth
/// picturing.
fn shoot<W: Widget>(
    out: &Path,
    name: &str,
    window: (WindowConfig, W),
    stage: impl FnOnce(&mut Driver),
) {
    let mut shot = Shot::open(window);
    shot.stage(stage);
    let chrome = shot.chrome();
    let snap = shot.backend.render_framed(&mut shot.root, &chrome);
    write_png(&out.join(format!("{name}.png")), &snap);
}

/// An example's window, started up offscreen.
struct Shot<W> {
    config: WindowConfig,
    backend: MockBackend,
    root: W,
}

impl<W: Widget> Shot<W> {
    /// Start `window` up the way the live runtime does: lay it out, then focus
    /// its first focusable widget.
    fn open((config, mut root): (WindowConfig, W)) -> Self {
        let mut backend = MockBackend::new(config.size.w, config.size.h)
            .with_scale(SCALE)
            // A display exactly as dense as a logical pixel aims for (1/72 in),
            // so the credit card example can draw its card true to size.
            .with_display_dpi(72.0 * SCALE);
        if let Some(font) = Font::load_sans() {
            backend = backend.with_sans_font(font);
        }
        if let Some(font) = Font::load_serif() {
            backend = backend.with_serif_font(font);
        }
        if let Some(font) = Font::load_monospace() {
            backend = backend.with_mono_font(font);
        }
        // A first render lays the tree out and lets widgets that cache
        // geometry while painting (menu-bar titles, list rows) get ready for
        // hit-testing.
        let _ = backend.render(&mut root);
        root.focus_first();
        Self {
            config,
            backend,
            root,
        }
    }

    /// Drive the window with synthetic input.
    fn stage(&mut self, stage: impl FnOnce(&mut Driver)) {
        stage(&mut Driver {
            backend: &self.backend,
            root: &mut self.root,
        });
    }

    /// The chrome a window manager gives a window like this one.
    fn chrome(&self) -> WindowChrome {
        let title = self.config.title.clone();
        if self.config.resizable {
            WindowChrome::resizable(title)
        } else {
            WindowChrome::fixed(title)
        }
    }

    /// Logical size of the window in its frame.
    fn frame(&self) -> Size {
        framed(self.config.size, if self.config.resizable { 4 } else { 1 })
    }

    /// Logical size of the dialog the window has open, in its frame.
    fn dialog_frame(&self) -> Option<Size> {
        let mut popups = Vec::new();
        self.root.collect_popups(&mut popups);
        let dialog = popups.iter().rfind(|p| p.kind == PopupKind::Dialog)?;
        Some(framed(Size::new(dialog.rect.w, dialog.rect.h), 4))
    }

    /// Place the window on `desktop`, inactive, its frame's top-left corner at
    /// `at`. A dialog it has open stays off the desktop until placed itself.
    fn place_inactive(&mut self, desktop: &mut Desktop, at: Point) {
        let chrome = self.chrome().inactive();
        self.backend
            .place_window(desktop, at, &mut self.root, &chrome);
    }
}

/// Canoe's frame around `content`: a `border` all round, and a 19 px title bar
/// with a 1 px line under it.
fn framed(content: Size, border: i32) -> Size {
    Size::new(content.w + 2 * border, content.h + 19 + 1 + 2 * border)
}

fn write_png(path: &Path, snap: &Snapshot) {
    std::fs::write(path, encode_png(snap))
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    println!("wrote {}", path.display());
}

/// Encode a framed window for the README. [`Snapshot::to_png`] favors speed,
/// which is right for test baselines; these images live in the repository, so
/// they are worth squeezing. A window on the desktop is opaque, so the alpha
/// channel goes, too.
fn encode_png(snap: &Snapshot) -> Vec<u8> {
    let rgb: Vec<u8> = snap
        .pixels()
        .iter()
        .flat_map(|&px| [(px >> 16) as u8, (px >> 8) as u8, px as u8])
        .collect();
    let mut buf = Vec::new();
    let mut encoder = png::Encoder::new(&mut buf, snap.width() as u32, snap.height() as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Best);
    encoder.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(&rgb).expect("png data");
    writer.finish().expect("png end");
    buf
}

/// Feeds synthetic input to a window about to be pictured.
struct Driver<'a> {
    backend: &'a MockBackend,
    root: &'a mut dyn Widget,
}

impl Driver<'_> {
    fn send(&mut self, event: Event) {
        self.backend.dispatch(self.root, &event);
    }

    fn press(&mut self, x: i32, y: i32, button: MouseButton, modifiers: Modifiers) {
        let pos = Point::new(x, y);
        self.send(Event::PointerDown {
            pos,
            button,
            modifiers,
        });
        self.send(Event::PointerUp {
            pos,
            button,
            modifiers,
        });
    }

    fn click(&mut self, x: i32, y: i32) {
        self.press(x, y, MouseButton::Left, Modifiers::default());
    }

    fn ctrl_click(&mut self, x: i32, y: i32) {
        let modifiers = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        self.press(x, y, MouseButton::Left, modifiers);
    }

    fn right_click(&mut self, x: i32, y: i32) {
        self.press(x, y, MouseButton::Right, Modifiers::default());
    }

    fn key(&mut self, key: NamedKey) {
        self.send(Event::KeyDown {
            key: Key::Named(key),
            modifiers: Modifiers::default(),
        });
    }

    /// A key that types `ch`, as a key press rather than text input.
    fn char_key(&mut self, ch: char) {
        self.send(Event::KeyDown {
            key: Key::Char(ch),
            modifiers: Modifiers::default(),
        });
    }

    fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.send(Event::Char {
                ch,
                modifiers: Modifiers::default(),
            });
        }
    }

    fn tick(&mut self) {
        self.send(Event::Tick);
    }

    /// Drop files from elsewhere onto the middle of the window.
    fn drop_files(&mut self, paths: &[&str]) {
        let size = self.root.bounds();
        let pos = Point::new(size.w / 2, size.h / 2);
        self.send(Event::DragEnter { pos });
        self.send(Event::Drop {
            pos,
            data: DragData::from_paths(paths.iter().map(PathBuf::from)),
        });
    }
}
