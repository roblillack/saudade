//! page_layout — numbers with spin buttons, and a radio choice of buttons.
//!
//! A page layout dialog with a live preview of the page:
//!
//! * the paper size comes from a [`Dropdown`], and the orientation from a
//!   [`ButtonGroup`] of two buttons with an icon and a label each, one of
//!   which is selected at a time;
//! * the margins are [`NumberInput`]s in millimetres with one decimal, stepped
//!   by half a millimetre; the scale goes in steps of 5 % and the number of
//!   copies in whole numbers, each limited to its own range. The margins and
//!   the scale show their unit inside the field.
//!
//! The number fields take nothing but numbers, and select theirs whenever they
//! take the focus, so typing replaces it. Type into them, click or hold
//! their arrows, press Up / Down or Page Up / Page Down, or turn the wheel over
//! the arrows: the preview follows every change. The arrow keys also move the
//! orientation along while it has the focus. Alt plus the underlined letter
//! jumps to a control. OK prints the settings and closes the window.

use std::cell::RefCell;
use std::rc::Rc;

use saudade::{
    App, Bevel, Button, ButtonGroup, Color, Container, Dropdown, FocusLabel, GroupButton, Label,
    NumberInput, Painter, Rect, SvgImage, Theme, Widget, WindowConfig, include_svg,
};

const PORTRAIT: SvgImage = include_svg!("examples/assets/icons/portrait.svg");
const LANDSCAPE: SvgImage = include_svg!("examples/assets/icons/landscape.svg");

const W: i32 = 464;
const H: i32 = 224;
/// The preview's box, in the window's right column.
const PREVIEW: Rect = Rect::new(320, 16, 128, 160);

struct Paper {
    name: &'static str,
    /// Width × height in millimetres, held upright.
    size: (f64, f64),
}

const PAPERS: [Paper; 5] = [
    Paper {
        name: "A5  (148 × 210 mm)",
        size: (148.0, 210.0),
    },
    Paper {
        name: "A4  (210 × 297 mm)",
        size: (210.0, 297.0),
    },
    Paper {
        name: "A3  (297 × 420 mm)",
        size: (297.0, 420.0),
    },
    Paper {
        name: "Letter  (8.5 × 11 in)",
        size: (215.9, 279.4),
    },
    Paper {
        name: "Legal  (8.5 × 14 in)",
        size: (215.9, 355.6),
    },
];

const ORIENTATIONS: [&str; 2] = ["Portrait", "Landscape"];

/// Everything the dialog sets, shared by the widgets that change it and the
/// ones that show it.
struct PageSetup {
    /// Index into [`PAPERS`].
    paper: usize,
    /// Index into [`ORIENTATIONS`].
    orientation: usize,
    /// Top, bottom, left, right, in millimetres.
    margins: [f64; 4],
    /// Percent.
    scale: f64,
    copies: u32,
}

impl PageSetup {
    /// The page's width and height in millimetres, as it lies.
    fn page_size(&self) -> (f64, f64) {
        let (w, h) = PAPERS[self.paper].size;
        if self.orientation == 1 {
            (h, w)
        } else {
            (w, h)
        }
    }
}

type Shared = Rc<RefCell<PageSetup>>;

fn main() {
    let (config, root) = window();
    App::new(config, root).with_theme(Theme::windows_31()).run();
}

/// The window and the widget tree inside it, kept apart from `main` so
/// `examples/screenshots.rs` can render the same window offscreen.
pub fn window() -> (WindowConfig, impl Widget) {
    let setup: Shared = Rc::new(RefCell::new(PageSetup {
        paper: 1,
        orientation: 0,
        margins: [25.0, 25.0, 20.0, 20.0],
        scale: 100.0,
        copies: 1,
    }));

    // A margin field: 0–50 mm in half-millimetre steps, shown with one
    // decimal. `side` is its index into `PageSetup::margins`.
    let margin = |x, y, side: usize| {
        let setup = setup.clone();
        let value = setup.borrow().margins[side];
        NumberInput::new(Rect::new(x, y, 72, 24))
            .with_range(0.0, 50.0)
            .with_step(0.5)
            .with_precision(1)
            .with_unit("mm")
            .with_value(value)
            .on_change(move |_cx, mm| setup.borrow_mut().margins[side] = mm)
    };

    let scale = NumberInput::new(Rect::new(72, 184, 72, 24))
        .with_unit("%")
        .with_range(25.0, 400.0)
        .with_step(5.0)
        .with_value(100.0)
        .on_change({
            let setup = setup.clone();
            move |_cx, percent| setup.borrow_mut().scale = percent
        });
    let copies = NumberInput::new(Rect::new(216, 184, 72, 24))
        .with_range(1.0, 99.0)
        .with_value(1.0)
        .on_change({
            let setup = setup.clone();
            move |_cx, n| setup.borrow_mut().copies = n as u32
        });

    let paper = Dropdown::new(Rect::new(104, 16, 184, 24))
        .with_items(PAPERS.iter().map(|paper| paper.name))
        .with_selected(setup.borrow().paper)
        .on_change({
            let setup = setup.clone();
            move |_cx, index| setup.borrow_mut().paper = index
        });

    // The buttons are in the order of `ORIENTATIONS`, so the index of the
    // selected one is the orientation.
    let orientation = ButtonGroup::horizontal(Rect::new(104, 48, 184, 24))
        .with_buttons([
            GroupButton::new(ORIENTATIONS[0]).with_icon(PORTRAIT),
            GroupButton::new(ORIENTATIONS[1]).with_icon(LANDSCAPE),
        ])
        .with_selected(setup.borrow().orientation)
        .on_change({
            let setup = setup.clone();
            move |_cx, index| setup.borrow_mut().orientation = index
        });

    let ok = Button::new(Rect::new(320, 184, 60, 24), "OK")
        .default(true)
        .on_click({
            let setup = setup.clone();
            move |cx| {
                print_settings(&setup.borrow());
                cx.close();
            }
        });
    let cancel = Button::new(Rect::new(388, 184, 60, 24), "Cancel").on_click(|cx| cx.close());

    let caption = |x, y, w, text| FocusLabel::new(Rect::new(x, y, w, 24), text);

    let root = Container::new(W, H)
        .add(caption(16, 16, 80, "&Paper:"))
        .add(paper)
        .add(caption(16, 48, 80, "&Orientation:"))
        .add(orientation)
        .add(Bevel::etched_line(16, 86, 272))
        .add(Label::new(Rect::new(16, 96, 272, 16), "Margins"))
        .add(caption(16, 120, 56, "&Top:"))
        .add(margin(72, 120, 0))
        .add(caption(160, 120, 56, "&Bottom:"))
        .add(margin(216, 120, 1))
        .add(caption(16, 152, 56, "&Left:"))
        .add(margin(72, 152, 2))
        .add(caption(160, 152, 56, "&Right:"))
        .add(margin(216, 152, 3))
        .add(caption(16, 184, 56, "&Scale:"))
        .add(scale)
        .add(caption(160, 184, 56, "&Copies:"))
        .add(copies)
        .add(Preview(setup.clone()))
        .add(ok)
        .add(cancel);

    (WindowConfig::new("Page Layout", W, H), root)
}

fn print_settings(setup: &PageSetup) {
    let [top, bottom, left, right] = setup.margins;
    println!(
        "{} copies of {}, {}, at {} %; margins {top:.1} / {bottom:.1} / {left:.1} / {right:.1} mm",
        setup.copies,
        PAPERS[setup.paper].name,
        ORIENTATIONS[setup.orientation].to_lowercase(),
        setup.scale,
    );
}

/// The page as set up, drawn to scale in [`PREVIEW`]: the sheet, a stack of
/// sheets behind it for more than one copy, the margins as a dotted frame, and
/// grey lines of text inside them, spaced by the scale.
struct Preview(Shared);

/// How far each sheet behind the front one peeks out, in logical pixels.
const STACK_OFFSET: i32 = 3;
/// Line pitch of the stand-in text at 100 %, in millimetres.
const LINE_PITCH_MM: f64 = 7.0;
/// Lengths of the stand-in lines, as fractions of the line width; a zero is a
/// blank line between paragraphs. Repeated down the page.
const LINES: [f64; 11] = [1.0, 0.96, 1.0, 0.62, 0.0, 1.0, 0.92, 1.0, 0.98, 0.35, 0.0];

impl Widget for Preview {
    fn bounds(&self) -> Rect {
        PREVIEW
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let setup = self.0.borrow();
        painter.fill_rect(PREVIEW, theme.face);
        painter.sunken_bevel(PREVIEW, theme.highlight, theme.shadow);
        painter.stroke_rect(PREVIEW, theme.border);

        // Fit the page, and the sheets stacked behind it, into the box.
        let behind = (setup.copies.min(3) as i32 - 1) * STACK_OFFSET;
        let room = PREVIEW.inset(10);
        let (page_w, page_h) = setup.page_size();
        let px_per_mm = f64::min(
            f64::from(room.w - behind) / page_w,
            f64::from(room.h - behind) / page_h,
        );
        let px = |mm: f64| (mm * px_per_mm).round() as i32;
        let (w, h) = (px(page_w), px(page_h));
        let page = Rect::new(
            room.x + (room.w - behind - w) / 2,
            room.y + (room.h - behind - h) / 2,
            w,
            h,
        );

        // Back to front, so the front sheet lands on top.
        for i in (0..=behind / STACK_OFFSET).rev() {
            let offset = i * STACK_OFFSET;
            let sheet = Rect::new(page.x + offset, page.y + offset, w, h);
            painter.fill_rect(Rect::new(sheet.x + 2, sheet.y + 2, w, h), Color::DARK_GRAY);
            painter.fill_rect(sheet, Color::WHITE);
            painter.stroke_rect(sheet, theme.border);
        }

        let [top, bottom, left, right] = setup.margins;
        let text = Rect::new(
            page.x + px(left),
            page.y + px(top),
            w - px(left) - px(right),
            h - px(top) - px(bottom),
        );
        if text.w <= 0 || text.h <= 0 {
            return;
        }
        let pitch = (LINE_PITCH_MM * setup.scale / 100.0 * px_per_mm).max(2.0);
        let mut y = f64::from(text.y) + pitch / 2.0;
        for length in LINES.iter().cycle() {
            if y >= f64::from(text.bottom()) {
                break;
            }
            let line_w = (f64::from(text.w) * length).round() as i32;
            painter.h_line(text.x, y as i32, line_w, Color::MID_GRAY);
            y += pitch;
        }
        painter.focus_rect(text.inset(-1), Color::MID_GRAY);
    }
}
