//! circle_drawer — 7GUIs task 6.
//!
//! "Build a frame containing an undo and redo button as well as a canvas area
//! underneath. Left-clicking inside an empty area inside the canvas will create
//! an unfilled circle with a fixed diameter whose center is the left-clicked
//! point. The circle nearest to the mouse pointer such that the distance from
//! its center to the pointer is less than its radius, if it exists, is filled
//! with the color gray. The gray circle is the selected circle C. Right-clicking
//! C will make a popup menu appear with one entry 'Adjust diameter..'. Clicking
//! on this entry will open another frame with a slider inside that adjusts the
//! diameter of C. Changes are applied immediately. Closing this frame will mark
//! the last diameter as significant for the undo/redo history. Every circle
//! manipulation that was done while the diameter adjustment frame was open
//! should be batched as a single undo/redo step. The undo button will undo the
//! last significant change. The redo button will reverse the latest undo until
//! more recent changes have been made."
//!
//! This is the example that exercises custom canvas painting (there is no
//! circle primitive — [`Canvas`] rasterizes outlines with a midpoint algorithm
//! and fills disks with horizontal spans), pointer-driven *hover* selection,
//! an undo/redo history, and a real modal dialog.
//!
//! The "another frame with a slider" is a genuine modal dialog ([`Modal`]): a
//! separate top-level window hosting a [`Slider`] over the selected circle's
//! diameter. While the dialog is open the slider edits a shared *working*
//! diameter that the canvas reads live (so the circle resizes immediately in
//! the main window); closing the dialog commits that diameter as a single
//! undo/redo step — so every drag made while it was open collapses into one
//! step, as required. The smaller right-click context menu stays an in-window
//! overlay drawn by the canvas.

use std::cell::RefCell;
use std::rc::Rc;

use saudade::{
    App, Button, Color, Column, Container, Event, EventCtx, Key, Modal, MouseButton, NamedKey,
    Painter, Point, PopupRequest, Rect, Size, Slider, Theme, Widget, WindowConfig,
};

const W: i32 = 480;
const H: i32 = 400;
/// The drawing surface, in window coordinates. Circle centers are stored in the
/// same space and clipped to this rectangle when painted.
const CANVAS_RECT: Rect = Rect::new(12, 48, 456, 340);

const DEFAULT_DIAMETER: i32 = 30;
const MIN_DIAMETER: i32 = 4;
const MAX_DIAMETER: i32 = 160;

/// Gray fill of the selected circle.
const SELECTED_FILL: Color = Color::rgb(0xC8, 0xC8, 0xC8);

const MENU_W: i32 = 140;
const MENU_H: i32 = 22;
/// Logical size of the diameter-adjustment dialog.
const DIALOG_W: i32 = 300;
const DIALOG_H: i32 = 120;

// Shared-handle aliases for the state the widgets pass around.
type SharedDoc = Rc<RefCell<Doc>>;
type SharedAdjust = Rc<RefCell<Option<Adjust>>>;
type SharedModalHandle = Rc<RefCell<Modal>>;

fn main() {
    let (_doc, _adjust, _modal, root) = build_app();
    App::new(WindowConfig::new("Circle Drawer", W, H), root)
        .with_theme(Theme::windows_31())
        .run();
}

/// Build the whole widget tree plus the shared state behind it. Returns the
/// shared handles too so tests can drive and inspect the same tree the mock
/// backend renders.
fn build_app() -> (SharedDoc, SharedAdjust, SharedModalHandle, Column) {
    let doc = Rc::new(RefCell::new(Doc::new()));
    // The live diameter being edited while the dialog is open (`None` when it
    // is closed). The dialog's slider writes it; the canvas reads it.
    let adjust: Rc<RefCell<Option<Adjust>>> = Rc::new(RefCell::new(None));

    // Closing the dialog commits the working diameter as one undo step.
    let modal = Rc::new(RefCell::new(Modal::new().on_dismiss({
        let doc = doc.clone();
        let adjust = adjust.clone();
        move |cx| {
            if let Some(a) = adjust.borrow_mut().take() {
                let current = doc.borrow().circles().get(a.target).map(|c| c.d);
                if current != Some(a.diameter) {
                    doc.borrow_mut().set_diameter(a.target, a.diameter);
                }
            }
            cx.request_paint();
        }
    })));

    let undo = ToolButton::new(
        Button::new(Rect::new(12, 12, 84, 26), "Undo").on_click({
            let doc = doc.clone();
            move |cx| {
                doc.borrow_mut().undo();
                cx.request_paint();
            }
        }),
        {
            let doc = doc.clone();
            move || doc.borrow().can_undo()
        },
    );

    let redo = ToolButton::new(
        Button::new(Rect::new(104, 12, 84, 26), "Redo").on_click({
            let doc = doc.clone();
            move |cx| {
                doc.borrow_mut().redo();
                cx.request_paint();
            }
        }),
        {
            let doc = doc.clone();
            move || doc.borrow().can_redo()
        },
    );

    let content = Container::new(W, H).add(undo).add(redo).add(Canvas::new(
        doc.clone(),
        adjust.clone(),
        modal.clone(),
    ));

    // The modal floats as an overlay so the runtime can host it in its own
    // top-level window via PopupRequest.
    let root = Column::new()
        .add_fill(content)
        .add_overlay(SharedModal(modal.clone()));

    (doc, adjust, modal, root)
}

// ============================================================================
// Doc — the scene plus its snapshot-based undo/redo history.
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
struct Circle {
    x: i32,
    y: i32,
    d: i32,
}

/// Whole-scene undo history. `history[index]` is the live scene; committing a
/// change truncates any redo tail and pushes a new snapshot.
struct Doc {
    history: Vec<Vec<Circle>>,
    index: usize,
}

impl Doc {
    fn new() -> Self {
        Self {
            history: vec![Vec::new()],
            index: 0,
        }
    }

    fn circles(&self) -> &[Circle] {
        &self.history[self.index]
    }

    /// Push `scene` as the new current snapshot, dropping any redo tail.
    fn commit(&mut self, scene: Vec<Circle>) {
        self.history.truncate(self.index + 1);
        self.history.push(scene);
        self.index += 1;
    }

    fn can_undo(&self) -> bool {
        self.index > 0
    }

    fn can_redo(&self) -> bool {
        self.index + 1 < self.history.len()
    }

    fn undo(&mut self) {
        if self.can_undo() {
            self.index -= 1;
        }
    }

    fn redo(&mut self) {
        if self.can_redo() {
            self.index += 1;
        }
    }

    /// Append a circle of the default diameter, centered at `(x, y)`.
    fn add_circle(&mut self, x: i32, y: i32) {
        let mut scene = self.circles().to_vec();
        scene.push(Circle {
            x,
            y,
            d: DEFAULT_DIAMETER,
        });
        self.commit(scene);
    }

    /// Commit a new diameter for circle `target` as one history step.
    fn set_diameter(&mut self, target: usize, d: i32) {
        let mut scene = self.circles().to_vec();
        if let Some(c) = scene.get_mut(target) {
            c.d = d;
        }
        self.commit(scene);
    }
}

/// The working diameter being edited by the open dialog.
struct Adjust {
    target: usize,
    diameter: i32,
}

/// The circle "nearest to the pointer such that the distance from its center to
/// the pointer is less than its radius", or `None`. Ties break toward the
/// closest center.
fn pick(circles: &[Circle], pointer: Option<Point>) -> Option<usize> {
    let p = pointer?;
    let mut best: Option<usize> = None;
    let mut best_d2 = i64::MAX;
    for (i, c) in circles.iter().enumerate() {
        let r = (c.d / 2) as i64;
        let dx = (p.x - c.x) as i64;
        let dy = (p.y - c.y) as i64;
        let d2 = dx * dx + dy * dy;
        if d2 < r * r && d2 < best_d2 {
            best_d2 = d2;
            best = Some(i);
        }
    }
    best
}

/// Diameter to *render* circle `index` with — the dialog's live working value
/// while that circle is being adjusted, otherwise its stored diameter.
fn render_diameter(index: usize, stored: i32, adjust: Option<&Adjust>) -> i32 {
    match adjust {
        Some(a) if a.target == index => a.diameter,
        _ => stored,
    }
}

/// The circle drawn gray: the one under adjustment, else the menu's target,
/// else the hover pick.
fn highlighted(
    circles: &[Circle],
    pointer: Option<Point>,
    menu: Option<&MenuState>,
    adjust: Option<&Adjust>,
) -> Option<usize> {
    if let Some(a) = adjust {
        Some(a.target)
    } else if let Some(m) = menu {
        Some(m.target)
    } else {
        pick(circles, pointer)
    }
}

// ============================================================================
// Canvas — the interactive drawing surface plus its in-window right-click menu.
// The diameter editor lives in a separate `Modal`, not here.
// ============================================================================

/// The right-click context menu, anchored at `at`, acting on circle `target`.
struct MenuState {
    at: Point,
    target: usize,
}

struct Canvas {
    doc: SharedDoc,
    adjust: SharedAdjust,
    modal: SharedModalHandle,
    /// Last pointer position inside the canvas, for hover selection.
    pointer: Option<Point>,
    menu: Option<MenuState>,
}

impl Canvas {
    fn new(doc: SharedDoc, adjust: SharedAdjust, modal: SharedModalHandle) -> Self {
        Self {
            doc,
            adjust,
            modal,
            pointer: None,
            menu: None,
        }
    }

    /// The menu box, clamped to stay inside the canvas.
    fn menu_rect(&self) -> Option<Rect> {
        let m = self.menu.as_ref()?;
        let c = CANVAS_RECT;
        let x = m.at.x.min(c.right() - MENU_W - 1).max(c.x);
        let y = m.at.y.min(c.bottom() - MENU_H - 1).max(c.y);
        Some(Rect::new(x, y, MENU_W, MENU_H))
    }

    fn menu_item_rect(&self) -> Rect {
        self.menu_rect()
            .map(|r| r.inset(1))
            .unwrap_or(Rect::new(0, 0, 0, 0))
    }

    /// Open the diameter dialog for `target`, seeding the working diameter from
    /// the circle's current size and handing a slider-bearing body to the modal.
    fn open_dialog(&mut self, target: usize) {
        let current = self
            .doc
            .borrow()
            .circles()
            .get(target)
            .map(|c| c.d)
            .unwrap_or(DEFAULT_DIAMETER);
        *self.adjust.borrow_mut() = Some(Adjust {
            target,
            diameter: current,
        });
        let body = DiameterBody::new(self.adjust.clone(), current);
        self.modal.borrow_mut().show(
            "Adjust diameter",
            Size::new(DIALOG_W, DIALOG_H),
            Box::new(body),
        );
    }

    fn event_menu(&mut self, event: &Event, ctx: &mut EventCtx) {
        match *event {
            Event::PointerMove { pos } => {
                self.pointer = Some(pos);
                ctx.request_paint();
            }
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } => {
                if self.menu_item_rect().contains(pos) {
                    let target = self.menu.take().unwrap().target;
                    self.open_dialog(target);
                } else {
                    self.menu = None;
                }
                ctx.request_paint();
            }
            Event::PointerDown {
                button: MouseButton::Right,
                ..
            } => {
                self.menu = None;
                ctx.request_paint();
            }
            _ => {}
        }
    }

    fn event_canvas(&mut self, event: &Event, ctx: &mut EventCtx) {
        match *event {
            Event::PointerMove { pos } => {
                // Only repaint when the hovered circle actually changes.
                let before = {
                    let d = self.doc.borrow();
                    pick(d.circles(), self.pointer)
                };
                self.pointer = Some(pos);
                let after = {
                    let d = self.doc.borrow();
                    pick(d.circles(), self.pointer)
                };
                if before != after {
                    ctx.request_paint();
                }
            }
            Event::PointerLeave => {
                if self.pointer.is_some() {
                    self.pointer = None;
                    ctx.request_paint();
                }
            }
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } => {
                self.pointer = Some(pos);
                self.doc.borrow_mut().add_circle(pos.x, pos.y);
                ctx.request_paint();
            }
            Event::PointerDown {
                pos,
                button: MouseButton::Right,
            } => {
                self.pointer = Some(pos);
                let target = {
                    let d = self.doc.borrow();
                    pick(d.circles(), Some(pos))
                };
                if let Some(target) = target {
                    self.menu = Some(MenuState { at: pos, target });
                    ctx.request_paint();
                }
            }
            _ => {}
        }
    }
}

impl Widget for Canvas {
    fn bounds(&self) -> Rect {
        CANVAS_RECT
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        let c = CANVAS_RECT;
        painter.fill_rect(c, Color::WHITE);
        painter.sunken_bevel(c, theme.highlight, theme.shadow);
        painter.stroke_rect(c, theme.border);

        let saved = painter.push_clip(c.inset(1));
        let doc = self.doc.borrow();
        let circles = doc.circles();
        let adjust = self.adjust.borrow();
        let hi = highlighted(circles, self.pointer, self.menu.as_ref(), adjust.as_ref());
        for (i, circle) in circles.iter().enumerate() {
            let r = render_diameter(i, circle.d, adjust.as_ref()) / 2;
            if Some(i) == hi {
                fill_disk(painter, circle.x, circle.y, r, SELECTED_FILL);
            }
            draw_circle_outline(painter, circle.x, circle.y, r, theme.text);
        }
        painter.restore_clip(saved);
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        // The right-click menu is an in-window overlay; nothing to draw onto a
        // separate popup surface (the dialog handles its own).
        if painter.is_popup_pass() {
            return;
        }
        if let Some(rect) = self.menu_rect() {
            painter.fill_rect(rect, theme.background);
            painter.stroke_rect(rect, theme.border);
            let item = rect.inset(1);
            let hovered = self.pointer.is_some_and(|p| item.contains(p));
            let (bg, fg) = if hovered {
                (theme.highlight_bg, theme.highlight_text)
            } else {
                (theme.background, theme.text)
            };
            painter.fill_rect(item, bg);
            let ty = item.y + (item.h - theme.font_size as i32) / 2;
            painter.text(item.x + 6, ty, "Adjust diameter..", theme.font_size, fg);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if self.menu.is_some() {
            self.event_menu(event, ctx);
        } else {
            self.event_canvas(event, ctx);
        }
    }

    fn captures_pointer(&self) -> bool {
        // Keep events flowing here while the menu is up so it behaves modally.
        // (The diameter dialog is a real modal window handled by the runtime.)
        self.menu.is_some()
    }
}

// ============================================================================
// DiameterBody — the modal dialog's content: a label, a diameter slider, and an
// OK button. Laid out into the dialog's client rect, so everything is placed
// relative to that rect.
// ============================================================================

struct DiameterBody {
    slider: Slider,
    rect: Rect,
}

impl DiameterBody {
    fn new(adjust: SharedAdjust, initial: i32) -> Self {
        let slider = Slider::new(Rect::new(0, 0, 10, 10), MIN_DIAMETER, MAX_DIAMETER)
            .with_value(initial)
            .on_change(move |cx, value| {
                if let Some(a) = adjust.borrow_mut().as_mut() {
                    a.diameter = value;
                }
                cx.request_paint();
            });
        Self {
            slider,
            rect: Rect::new(0, 0, 0, 0),
        }
    }

    fn slider_area(&self) -> Rect {
        Rect::new(self.rect.x + 16, self.rect.y + 44, self.rect.w - 32, 20)
    }

    fn ok_rect(&self) -> Rect {
        Rect::new(
            self.rect.x + (self.rect.w - 64) / 2,
            self.rect.bottom() - 12 - 26,
            64,
            26,
        )
    }
}

impl Widget for DiameterBody {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
        let area = self.slider_area();
        self.slider.set_rect(area);
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        painter.text(
            self.rect.x + 16,
            self.rect.y + 14,
            "Adjust diameter:",
            theme.font_size,
            theme.text,
        );
        self.slider.paint(painter, theme);
        let ok = self.ok_rect();
        painter.button(ok, theme, false, true);
        painter.text_centered(ok, "OK", theme.font_size, theme.text);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        match *event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } if self.ok_rect().contains(pos) => {
                ctx.request_dismiss();
            }
            // Enter confirms; Escape is handled by the hosting `Modal`.
            Event::KeyDown {
                key: Key::Named(NamedKey::Enter),
                ..
            } => {
                ctx.request_dismiss();
            }
            // Everything else drives the slider (drag, and arrow keys when it
            // holds focus).
            _ => self.slider.event(event, ctx),
        }
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.slider.set_focused(focused);
    }

    fn captures_pointer(&self) -> bool {
        self.slider.captures_pointer()
    }
}

/// Midpoint-circle outline at logical center `(cx, cy)`, radius `r`.
fn draw_circle_outline(painter: &mut Painter, cx: i32, cy: i32, r: i32, color: Color) {
    if r <= 0 {
        painter.pixel(cx, cy, color);
        return;
    }
    let mut x = r;
    let mut y = 0;
    let mut err = 1 - r;
    while x >= y {
        for (px, py) in [
            (cx + x, cy + y),
            (cx - x, cy + y),
            (cx + x, cy - y),
            (cx - x, cy - y),
            (cx + y, cy + x),
            (cx - y, cy + x),
            (cx + y, cy - x),
            (cx - y, cy - x),
        ] {
            painter.pixel(px, py, color);
        }
        y += 1;
        if err < 0 {
            err += 2 * y + 1;
        } else {
            x -= 1;
            err += 2 * (y - x) + 1;
        }
    }
}

/// Filled disk drawn as horizontal spans — the gray selection fill.
fn fill_disk(painter: &mut Painter, cx: i32, cy: i32, r: i32, color: Color) {
    if r <= 0 {
        return;
    }
    let r2 = (r as i64) * (r as i64);
    for dy in -r..=r {
        let dy2 = (dy as i64) * (dy as i64);
        let half = ((r2 - dy2) as f64).sqrt() as i32;
        painter.h_line(cx - half, cy + dy, half * 2 + 1, color);
    }
}

// ============================================================================
// ToolButton — a Button that pulls its enabled flag from a predicate (here,
// `Doc::can_undo` / `can_redo`). Same pattern as the crud example: the buttons
// reflect the live history state without anyone pushing into them, and they
// never mutate themselves from inside their own click handler.
// ============================================================================

struct ToolButton {
    button: Button,
    enabled: Box<dyn Fn() -> bool>,
}

impl ToolButton {
    fn new(button: Button, enabled: impl Fn() -> bool + 'static) -> Self {
        Self {
            button,
            enabled: Box::new(enabled),
        }
    }

    fn sync(&mut self) {
        let enabled = (self.enabled)();
        self.button.set_enabled(enabled);
    }
}

impl Widget for ToolButton {
    fn bounds(&self) -> Rect {
        self.button.bounds()
    }
    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.sync();
        self.button.paint(painter, theme);
    }
    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.sync();
        self.button.event(event, ctx);
    }
    fn captures_pointer(&self) -> bool {
        self.button.captures_pointer()
    }
    fn focusable(&self) -> bool {
        (self.enabled)()
    }
    fn set_focused(&mut self, focused: bool) {
        self.button.set_focused(focused);
    }
    fn layout(&mut self, bounds: Rect) {
        self.button.layout(bounds);
    }
}

// ============================================================================
// SharedModal — lets the canvas's callbacks and the widget tree hold the same
// live `Modal`. Mirrors the SharedDialog adapter in the picker example.
// ============================================================================

struct SharedModal(SharedModalHandle);

impl Widget for SharedModal {
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
    fn accepts_accelerators(&self) -> bool {
        self.0.borrow().accepts_accelerators()
    }
    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }
    fn popup_request(&self) -> Option<PopupRequest> {
        self.0.borrow().popup_request()
    }
    fn wants_ticks(&self) -> bool {
        self.0.borrow().wants_ticks()
    }
}

// ============================================================================
// Tests — the history and hit-testing logic is pure, so it runs headless and
// doubles as documentation of the 7GUIs undo/redo and selection rules.
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn circle(x: i32, y: i32, d: i32) -> Circle {
        Circle { x, y, d }
    }

    #[test]
    fn create_then_undo_redo_walks_the_history() {
        let mut doc = Doc::new();
        assert!(!doc.can_undo() && !doc.can_redo());

        doc.add_circle(100, 100);
        doc.add_circle(200, 150);
        assert_eq!(doc.circles().len(), 2);
        assert!(doc.can_undo() && !doc.can_redo());

        doc.undo();
        assert_eq!(doc.circles().len(), 1);
        doc.undo();
        assert_eq!(doc.circles().len(), 0);
        assert!(!doc.can_undo() && doc.can_redo());

        doc.redo();
        doc.redo();
        assert_eq!(doc.circles().len(), 2);
        assert!(!doc.can_redo());
    }

    #[test]
    fn a_new_change_drops_the_redo_tail() {
        let mut doc = Doc::new();
        doc.add_circle(100, 100);
        doc.add_circle(200, 150);
        doc.undo(); // back to 1 circle, redo available
        assert!(doc.can_redo());
        doc.add_circle(300, 300); // diverge
        assert!(!doc.can_redo());
        assert_eq!(doc.circles().len(), 2);
    }

    #[test]
    fn diameter_adjustment_is_one_undoable_step() {
        let mut doc = Doc::new();
        doc.add_circle(100, 100); // diameter == DEFAULT_DIAMETER
        let before = doc.circles()[0].d;
        doc.set_diameter(0, 90);
        assert_eq!(doc.circles()[0].d, 90);
        doc.undo();
        assert_eq!(doc.circles()[0].d, before); // one step back to the old size
        assert_eq!(doc.circles().len(), 1); // the circle itself is still there
    }

    #[test]
    fn pick_selects_the_nearest_circle_within_its_radius() {
        let circles = [circle(100, 100, 40), circle(160, 100, 40)];
        // Inside the first circle only.
        assert_eq!(pick(&circles, Some(Point::new(105, 100))), Some(0));
        // In the overlap region, nearer the second center.
        assert_eq!(pick(&circles, Some(Point::new(150, 100))), Some(1));
        // Outside every radius.
        assert_eq!(pick(&circles, Some(Point::new(300, 300))), None);
        // No pointer at all.
        assert_eq!(pick(&circles, None), None);
    }

    // Drive the whole create → hover → right-click menu → modal dialog →
    // slider drag → dismiss → undo flow through the mock backend, rendering at
    // each step. This exercises the in-window menu overlay, the real modal
    // dialog (its popup-pass painting and event routing through `Modal`), the
    // shared working-diameter state, and the commit-on-close undo batching.
    #[test]
    fn interactive_flow_renders_without_panicking() {
        use saudade::mock::MockBackend;

        fn down(x: i32, y: i32, button: MouseButton) -> Event {
            Event::PointerDown {
                pos: Point::new(x, y),
                button,
            }
        }

        let (doc, adjust, modal, mut root) = build_app();
        let backend = MockBackend::new(W, H);
        backend.render(&mut root);

        // Create a circle, then hover its center.
        backend.dispatch(&mut root, &down(200, 200, MouseButton::Left));
        backend.dispatch(
            &mut root,
            &Event::PointerMove {
                pos: Point::new(200, 200),
            },
        );
        backend.render(&mut root);
        assert_eq!(doc.borrow().circles().len(), 1);

        // Right-click the circle → context menu; click its item → modal dialog.
        backend.dispatch(&mut root, &down(200, 200, MouseButton::Right));
        backend.render(&mut root);
        backend.dispatch(&mut root, &down(210, 210, MouseButton::Left));
        backend.render(&mut root); // renders the dialog via the popup pass
        assert!(modal.borrow().is_open());
        assert!(adjust.borrow().is_some());

        // Drag the dialog's slider (it lives at the dialog's centered rect).
        // 90 + (400-120)/2 = 90,140 origin; the slider spans the body width.
        backend.dispatch(&mut root, &down(200, 189, MouseButton::Left));
        backend.dispatch(
            &mut root,
            &Event::PointerMove {
                pos: Point::new(250, 189),
            },
        );
        backend.dispatch(
            &mut root,
            &Event::PointerUp {
                pos: Point::new(250, 189),
                button: MouseButton::Left,
            },
        );
        backend.render(&mut root);

        // Close the dialog with Escape → commits the new diameter as one step.
        backend.dispatch(
            &mut root,
            &Event::KeyDown {
                key: Key::Named(NamedKey::Escape),
                modifiers: Default::default(),
            },
        );
        backend.render(&mut root);
        assert!(!modal.borrow().is_open());
        {
            let d = doc.borrow();
            assert_eq!(d.circles().len(), 1);
            let diameter = d.circles()[0].d;
            assert_ne!(diameter, DEFAULT_DIAMETER);
            assert!((MIN_DIAMETER..=MAX_DIAMETER).contains(&diameter));
            assert!(d.can_undo());
        }

        // Undo the diameter change (button fires on release).
        backend.dispatch(&mut root, &down(54, 25, MouseButton::Left));
        backend.dispatch(
            &mut root,
            &Event::PointerUp {
                pos: Point::new(54, 25),
                button: MouseButton::Left,
            },
        );
        backend.render(&mut root);
        let d = doc.borrow();
        assert_eq!(d.circles().len(), 1);
        assert_eq!(d.circles()[0].d, DEFAULT_DIAMETER);
        assert!(d.can_redo());
    }
}
