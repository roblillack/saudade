use crate::event::{Event, EventCtx, Key, MouseButton, NamedKey};
use crate::geometry::{Point, Rect};
use crate::painter::{Merged, Painter};
use crate::svg::SvgImage;
use crate::theme::Theme;
use crate::widget::Widget;
use crate::widgets::button::paint_face;
use crate::widgets::scrollbar::Orientation;

type ChangeHandler = Box<dyn FnMut(&mut EventCtx, usize)>;

/// One button of a [`ButtonGroup`]: a label, an icon, or both, drawn the way a
/// [`Button`](crate::widgets::Button) draws its own.
#[derive(Clone)]
pub struct GroupButton {
    label: String,
    icon: Option<SvgImage>,
}

impl GroupButton {
    /// A button with a label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
        }
    }

    /// A button with an icon and no label.
    pub fn icon(icon: SvgImage) -> Self {
        Self::new("").with_icon(icon)
    }

    /// Lead the label with `icon`. As on a [`Button`](crate::widgets::Button),
    /// it is drawn tinted in the label's color, so it greys out with it — which
    /// makes it meant for single-color glyphs whose baked color is just a
    /// placeholder.
    pub fn with_icon(mut self, icon: SvgImage) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

impl From<&str> for GroupButton {
    fn from(label: &str) -> Self {
        Self::new(label)
    }
}

impl From<String> for GroupButton {
    fn from(label: String) -> Self {
        Self::new(label)
    }
}

/// A row (or column) of buttons joined into one control, of which one is
/// selected at a time — a radio choice made of push buttons, such as a page's
/// orientation or a paragraph's alignment. Each button shows a label, an icon,
/// or both (see [`GroupButton`]).
///
/// The buttons share the rect evenly, and the borders between them. The
/// selected one stays pushed in, its face stippled the way a latched toolbar
/// button's is. Clicking a button selects it, once the mouse is released over
/// it. While the group has focus, the arrow keys select the previous or next
/// button and Home / End the first or last. Either way,
/// [`on_change`](Self::on_change) hears about it.
pub struct ButtonGroup {
    rect: Rect,
    orientation: Orientation,
    buttons: Vec<GroupButton>,
    selected: Option<usize>,
    /// The button a mouse press went down on, while the press lasts.
    pressed: Option<usize>,
    /// Whether the pointer is still over the pressed button. The button only
    /// looks pushed in, and only gets selected on release, while it is.
    armed: bool,
    focused: bool,
    enabled: bool,
    on_change: Option<ChangeHandler>,
}

impl ButtonGroup {
    pub fn new(rect: Rect, orientation: Orientation) -> Self {
        Self {
            rect,
            orientation,
            buttons: Vec::new(),
            selected: None,
            pressed: None,
            armed: false,
            focused: false,
            enabled: true,
            on_change: None,
        }
    }

    /// Buttons side by side, left to right.
    pub fn horizontal(rect: Rect) -> Self {
        Self::new(rect, Orientation::Horizontal)
    }

    /// Buttons stacked, top to bottom.
    pub fn vertical(rect: Rect) -> Self {
        Self::new(rect, Orientation::Vertical)
    }

    /// Set the buttons. Takes [`GroupButton`]s, or plain labels: both
    /// `["Left", "Center", "Right"]` and a list of `GroupButton`s work. The
    /// first one becomes the selection.
    pub fn with_buttons<I, B>(mut self, buttons: I) -> Self
    where
        I: IntoIterator<Item = B>,
        B: Into<GroupButton>,
    {
        self.set_buttons(buttons.into_iter().map(Into::into).collect());
        self
    }

    /// Replace every button. The selection is kept if it still points at a
    /// button; otherwise it falls back to the first one (or to none when there
    /// are no buttons left).
    pub fn set_buttons(&mut self, buttons: Vec<GroupButton>) {
        self.buttons = buttons;
        self.selected = match self.selected {
            Some(i) if i < self.buttons.len() => Some(i),
            _ if self.buttons.is_empty() => None,
            _ => Some(0),
        };
        self.pressed = None;
    }

    pub fn buttons(&self) -> &[GroupButton] {
        &self.buttons
    }

    pub fn with_selected(mut self, index: usize) -> Self {
        self.set_selected(Some(index));
        self
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Select a button by index, or none with `None`; an index past the last
    /// button selects none. Doesn't call `on_change`, so programmatic updates
    /// can't loop back through it.
    pub fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.filter(|&i| i < self.buttons.len());
    }

    /// Call `handler` with the index of the button the user selected, whenever
    /// that is a different one than before.
    pub fn on_change<F>(mut self, handler: F) -> Self
    where
        F: FnMut(&mut EventCtx, usize) + 'static,
    {
        self.on_change = Some(Box::new(handler));
        self
    }

    /// Install (or replace) the change handler after construction, for callers
    /// that wire it up once the widgets it talks to exist.
    pub fn set_on_change<F>(&mut self, handler: F)
    where
        F: FnMut(&mut EventCtx, usize) + 'static,
    {
        self.on_change = Some(Box::new(handler));
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.set_enabled(enabled);
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable every button. Disabled, their labels and icons are
    /// engraved like a disabled `Button`'s, the selected one still shows, and
    /// the group takes no focus or input.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.pressed = None;
        }
    }

    /// The rect of button `index`. The buttons split the group's length
    /// evenly and overlap by one logical pixel, so the line between two is one
    /// border both share.
    fn button_rect(&self, index: usize) -> Rect {
        let n = self.buttons.len().max(1) as i32;
        let i = index as i32;
        let r = self.rect;
        match self.orientation {
            Orientation::Horizontal => {
                let edge = |i: i32| r.x + i * (r.w - 1) / n;
                Rect::new(edge(i), r.y, edge(i + 1) - edge(i) + 1, r.h)
            }
            Orientation::Vertical => {
                let edge = |i: i32| r.y + i * (r.h - 1) / n;
                Rect::new(r.x, edge(i), r.w, edge(i + 1) - edge(i) + 1)
            }
        }
    }

    /// The edges button `index` shares with the one before it.
    fn merged(&self, index: usize) -> Merged {
        match (index, self.orientation) {
            (0, _) => Merged::NONE,
            (_, Orientation::Horizontal) => Merged::LEFT,
            (_, Orientation::Vertical) => Merged::TOP,
        }
    }

    fn hit(&self, pos: Point) -> Option<usize> {
        (0..self.buttons.len()).find(|&i| self.button_rect(i).contains(pos))
    }

    /// Select button `index`, telling the change handler if it wasn't already.
    fn select(&mut self, index: usize, ctx: &mut EventCtx) {
        if index >= self.buttons.len() || self.selected == Some(index) {
            return;
        }
        self.selected = Some(index);
        if let Some(handler) = self.on_change.as_mut() {
            handler(ctx, index);
        }
    }

    fn handle_key(&mut self, key: &Key, ctx: &mut EventCtx) {
        let Some(last) = self.buttons.len().checked_sub(1) else {
            return;
        };
        let current = self.selected.unwrap_or(0);
        let target = match key {
            Key::Named(NamedKey::Left | NamedKey::Up) => current.saturating_sub(1),
            Key::Named(NamedKey::Right | NamedKey::Down) => (current + 1).min(last),
            Key::Named(NamedKey::Home) => 0,
            Key::Named(NamedKey::End) => last,
            _ => return,
        };
        self.select(target, ctx);
        ctx.request_paint();
        ctx.consume_event();
    }
}

impl Widget for ButtonGroup {
    fn bounds(&self) -> Rect {
        self.rect
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        for (i, button) in self.buttons.iter().enumerate() {
            let rect = self.button_rect(i);
            let merged = self.merged(i);
            let latched = self.selected == Some(i);
            let down = latched || (self.pressed == Some(i) && self.armed);
            // The scrollbar's light button chrome, as on a `SpinButton`: square
            // corners that join into one frame. Later buttons overlap the one
            // before by a pixel and mark that edge merged, so the two borders
            // land on the same device pixels at every scale.
            painter.light_button_merged(rect, merged, theme, down);
            if latched {
                // Stipple the face of the selected button with the highlight,
                // inside the line of shadow along its top and left.
                painter.crisp_merged(rect, merged, |p, f| {
                    let inner = f.inside(1);
                    let shadow = f.bevel_ring(1, 0);
                    let (x, y) = (shadow.left.right(), shadow.top.bottom());
                    let face = Rect::new(x, y, inner.right() - x, inner.bottom() - y);
                    p.fill_checker_cells(face, theme.face, theme.highlight, f.depth(1));
                });
            }

            let saved = painter.push_clip_frame(rect, 2, merged);
            // A pushed-in button carries its face a pixel down and right.
            let nudge = i32::from(down);
            let face = Rect::new(rect.x + nudge, rect.y + nudge, rect.w, rect.h);
            paint_face(
                painter,
                face,
                &button.label,
                button.icon.as_ref(),
                self.enabled,
                theme,
            );
            painter.restore_clip(saved);
        }

        // The dotted focus rectangle sits inside the selected button, where
        // the arrow keys start from.
        if self.focused && self.enabled && !self.buttons.is_empty() {
            let rect = self.button_rect(self.selected.unwrap_or(0));
            painter.focus_rect(rect.inset(3), theme.text);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if !self.enabled {
            return;
        }
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(i) = self.hit(*pos) {
                    self.pressed = Some(i);
                    self.armed = true;
                    ctx.request_focus();
                    ctx.request_paint();
                }
            }
            // Sliding off the pressed button pops it back out; sliding back on
            // pushes it in again.
            Event::PointerMove { pos } => {
                if let Some(i) = self.pressed {
                    let armed = self.hit(*pos) == Some(i);
                    if armed != self.armed {
                        self.armed = armed;
                        ctx.request_paint();
                    }
                }
            }
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(i) = self.pressed.take() {
                    if self.armed && self.hit(*pos) == Some(i) {
                        self.select(i, ctx);
                    }
                    self.armed = false;
                    ctx.request_paint();
                }
            }
            Event::PointerLeave if self.pressed.is_some() && self.armed => {
                self.armed = false;
                ctx.request_paint();
            }
            Event::KeyDown { key, modifiers } if self.focused && !modifiers.has_command() => {
                self.handle_key(key, ctx);
            }
            _ => {}
        }
    }

    fn captures_pointer(&self) -> bool {
        self.pressed.is_some()
    }

    fn focusable(&self) -> bool {
        self.enabled && !self.buttons.is_empty()
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn layout(&mut self, bounds: Rect) {
        self.rect = bounds;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::event::Modifiers;

    /// Three buttons side by side, 151 px wide in all — so each is 51 px,
    /// sharing a border with its neighbour — and the indices the change
    /// handler is called with.
    fn group() -> (ButtonGroup, Rc<RefCell<Vec<usize>>>) {
        let changes = Rc::new(RefCell::new(Vec::new()));
        let sink = changes.clone();
        let group = ButtonGroup::horizontal(Rect::new(0, 0, 151, 24))
            .with_buttons(["Left", "Center", "Right"])
            .on_change(move |_cx, i| sink.borrow_mut().push(i));
        (group, changes)
    }

    fn center(r: Rect) -> Point {
        Point::new(r.x + r.w / 2, r.y + r.h / 2)
    }

    fn send(group: &mut ButtonGroup, event: Event) -> EventCtx {
        let mut ctx = EventCtx::new();
        group.event(&event, &mut ctx);
        ctx
    }

    fn press(group: &mut ButtonGroup, pos: Point) -> EventCtx {
        send(
            group,
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        )
    }

    fn release(group: &mut ButtonGroup, pos: Point) {
        send(
            group,
            Event::PointerUp {
                pos,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            },
        );
    }

    fn click(group: &mut ButtonGroup, index: usize) {
        let at = center(group.button_rect(index));
        press(group, at);
        release(group, at);
    }

    fn key(group: &mut ButtonGroup, key: NamedKey) -> EventCtx {
        send(
            group,
            Event::KeyDown {
                key: Key::Named(key),
                modifiers: Modifiers::default(),
            },
        )
    }

    #[test]
    fn the_buttons_split_the_rect_and_share_their_borders() {
        let (group, _) = group();
        let rects: Vec<Rect> = (0..3).map(|i| group.button_rect(i)).collect();
        assert_eq!(rects[0].x, 0);
        assert_eq!(rects[2].right(), 151);
        for pair in rects.windows(2) {
            assert_eq!(pair[0].right() - 1, pair[1].x, "one shared border");
        }
        assert!(rects.iter().all(|r| r.w == 51), "evenly: {rects:?}");

        let column = ButtonGroup::vertical(Rect::new(0, 0, 80, 49)).with_buttons(["a", "b"]);
        assert_eq!(column.button_rect(0), Rect::new(0, 0, 80, 25));
        assert_eq!(column.button_rect(1), Rect::new(0, 24, 80, 25));
    }

    #[test]
    fn the_first_button_starts_out_selected() {
        let (group, _) = group();
        assert_eq!(group.selected(), Some(0));
        assert!(
            ButtonGroup::horizontal(Rect::new(0, 0, 10, 10))
                .selected()
                .is_none()
        );
    }

    #[test]
    fn a_click_selects_its_button_on_release() {
        let (mut group, changes) = group();
        let at = center(group.button_rect(2));
        let ctx = press(&mut group, at);
        assert!(ctx.is_focus_requested());
        assert!(group.captures_pointer());
        assert_eq!(group.selected(), Some(0), "not yet, on the press");
        release(&mut group, at);
        assert_eq!(group.selected(), Some(2));
        assert!(!group.captures_pointer());
        click(&mut group, 2);
        assert_eq!(
            *changes.borrow(),
            [2],
            "the selected one again is no change"
        );
    }

    #[test]
    fn releasing_off_the_pressed_button_selects_nothing() {
        let (mut group, changes) = group();
        let pressed = center(group.button_rect(1));
        press(&mut group, pressed);
        let elsewhere = center(group.button_rect(2));
        send(&mut group, Event::PointerMove { pos: elsewhere });
        assert!(!group.armed, "sliding off pops the button back out");
        release(&mut group, elsewhere);
        assert_eq!(group.selected(), Some(0));
        assert!(changes.borrow().is_empty());
    }

    #[test]
    fn arrow_keys_move_the_selection_while_focused() {
        let (mut group, changes) = group();
        key(&mut group, NamedKey::Right);
        assert_eq!(group.selected(), Some(0), "not without the focus");

        group.set_focused(true);
        let ctx = key(&mut group, NamedKey::Right);
        assert!(ctx.is_consumed());
        key(&mut group, NamedKey::Down);
        key(&mut group, NamedKey::Right);
        assert_eq!(group.selected(), Some(2), "stopping at the last");
        key(&mut group, NamedKey::Home);
        key(&mut group, NamedKey::Left);
        key(&mut group, NamedKey::End);
        assert_eq!(*changes.borrow(), [1, 2, 0, 2]);
    }

    #[test]
    fn set_selected_does_not_call_the_handler() {
        let (mut group, changes) = group();
        group.set_selected(Some(1));
        assert_eq!(group.selected(), Some(1));
        group.set_selected(Some(7));
        assert_eq!(group.selected(), None, "past the last button");
        assert!(changes.borrow().is_empty());
    }

    #[test]
    fn a_disabled_group_takes_nothing() {
        let (group, changes) = group();
        let mut group = group.with_enabled(false);
        assert!(!group.focusable());
        click(&mut group, 1);
        assert_eq!(group.selected(), Some(0));
        assert!(changes.borrow().is_empty());
    }
}
