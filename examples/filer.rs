//! filer — a tiny saudade demo that walks the local filesystem inside a
//! list widget. Double-click (or Enter) on a directory descends into it;
//! double-click on `..` ascends to the parent. Ctrl/Shift+click selects
//! several entries at once; drag the selection out of the window to drop the
//! files onto another application (Wayland only — see `EventCtx::start_drag`).

use std::path::{Path, PathBuf};

use saudade::{
    App, DragData, Event, EventCtx, List, ListItem, MouseButton, Painter, Point, Rect, SvgImage,
    Theme, Widget, WindowConfig, include_svg,
};

const WINDOW_W: i32 = 420;
const WINDOW_H: i32 = 360;
const HEADER_H: i32 = 22;
/// How far (logical px) the pointer must travel from a press before we treat it
/// as a drag-out rather than a click — the usual click-vs-drag dead zone.
const DRAG_THRESHOLD: i64 = 5;

// Row icons, baked from SVG at compile time via `include_svg!`. These are the
// same asset files saudade's own `FileDialog` uses, so the file manager and the
// Open / Save dialogs share one set of folder / file / up-arrow marks.
const FOLDER_ICON: SvgImage = include_svg!("assets/icons/folder.svg");
const FILE_ICON: SvgImage = include_svg!("assets/icons/file.svg");
const UP_ICON: SvgImage = include_svg!("assets/icons/up.svg");

fn main() {
    let start = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"));
    let start = start.canonicalize().unwrap_or_else(|_| PathBuf::from("/"));

    let browser = FileBrowser::new(start);

    App::new(
        WindowConfig::new("File Manager", WINDOW_W, WINDOW_H).resizable(true),
        browser,
    )
    .with_theme(Theme::windows_31())
    .run();
}

// ============================================================================
// FileBrowser — paints a header strip with the current path and hosts a List
// underneath. The widget owns the path state, so navigation is just a matter
// of reloading items into the list when activation fires.
// ============================================================================

struct FileBrowser {
    list: List,
    path: PathBuf,
    bounds: Rect,
    /// Where a press that may turn into a drag-out started. Set on a left press
    /// over a real entry, cleared on release/leave or once the pointer moves far
    /// enough to actually start the drag. The drag carries the whole selection,
    /// so only the start point is remembered — not a single index.
    drag_armed: Option<Point>,
}

impl FileBrowser {
    fn new(path: PathBuf) -> Self {
        let mut me = Self {
            list: List::new(Rect::new(0, 0, 0, 0)).with_multi_select(true),
            path,
            bounds: Rect::new(0, 0, 0, 0),
            drag_armed: None,
        };
        me.reload();
        me
    }

    fn reload(&mut self) {
        let entries = read_entries(&self.path);
        let mut items = Vec::with_capacity(entries.len() + 1);
        if self.path.parent().is_some() {
            items.push(ListItem::new("..").with_svg_icon(UP_ICON));
        }
        for entry in entries {
            let icon = if entry.is_dir { FOLDER_ICON } else { FILE_ICON };
            items.push(ListItem::new(entry.name).with_svg_icon(icon));
        }
        self.list.set_items(items);
        self.list.set_selected(Some(0));
    }

    fn descend(&mut self, name: &str) {
        if name == ".." {
            if let Some(parent) = self.path.parent() {
                self.path = parent.to_path_buf();
                self.reload();
            }
            return;
        }
        let target = self.path.join(name);
        if target.is_dir() {
            self.path = target;
            self.reload();
        }
    }

    fn handle_activation(&mut self, ctx: &mut EventCtx) {
        let Some(idx) = self.list.take_activated() else {
            return;
        };
        let name = self.list.items().get(idx).map(|i| i.label.clone());
        if let Some(name) = name {
            self.descend(&name);
            ctx.request_paint();
        }
    }

    /// Arm a possible drag-out after a left press the list has already turned
    /// into a selection. We only arm over a real entry inside the list — never
    /// `..` (dragging "go up" makes no sense), the header strip, or the
    /// scrollbar gutter. That last exclusion matters: the scrollbar sits
    /// *inside* the list bounds, so without it grabbing the thumb would both
    /// scroll and arm a drag — and the drag wins on the next move, yanking a
    /// file out instead of scrolling.
    fn arm_drag(&mut self, pos: Point) {
        self.drag_armed = None;
        if !self.list.bounds().contains(pos) || self.list.scrollbar_hit(pos) {
            return;
        }
        if let Some(idx) = self.list.selected_index()
            && self.list.items().get(idx).is_some_and(|i| i.label != "..")
        {
            self.drag_armed = Some(pos);
        }
    }

    /// If an armed press has dragged past the dead zone, hand the absolute paths
    /// of every selected entry (skipping `..`) to the runtime as a drag-and-drop
    /// payload and disarm.
    fn maybe_start_drag(&mut self, pos: Point, ctx: &mut EventCtx) {
        let Some(start) = self.drag_armed else {
            return;
        };
        let (dx, dy) = ((pos.x - start.x) as i64, (pos.y - start.y) as i64);
        if dx * dx + dy * dy < DRAG_THRESHOLD * DRAG_THRESHOLD {
            return;
        }
        self.drag_armed = None;
        let paths: Vec<PathBuf> = self
            .list
            .selected_indices()
            .iter()
            .filter_map(|&i| self.list.items().get(i))
            .filter(|item| item.label != "..")
            .map(|item| self.path.join(&item.label))
            .collect();
        if !paths.is_empty() {
            ctx.start_drag(DragData::from_paths(paths));
        }
    }
}

impl Widget for FileBrowser {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        // No background fill — the window background (pattern) shows behind the
        // header strip; the list paints itself sunken-white over the top.
        let header_rect = Rect::new(self.bounds.x, self.bounds.y, self.bounds.w, HEADER_H);
        painter.text(
            header_rect.x + 8,
            header_rect.y + 5,
            &self.path.display().to_string(),
            theme.font_size,
            theme.text,
        );
        self.list.paint(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // Drag-out gesture: a left press over an entry arms it, and enough
        // motion before release turns it into a drag. Plain clicks (no motion)
        // and double-clicks to descend still fall through to the list normally.
        if let Event::PointerMove { pos } = event {
            self.maybe_start_drag(*pos, ctx);
        }
        self.list.event(event, ctx);
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
                ..
            } => self.arm_drag(*pos),
            Event::PointerUp { .. } | Event::PointerLeave => self.drag_armed = None,
            _ => {}
        }
        self.handle_activation(ctx);
    }

    fn captures_pointer(&self) -> bool {
        self.list.captures_pointer()
    }

    // Focus is owned by the inner list — FileBrowser is just a wrapper.
    // Delegating these three keeps focus handling transparent: parent
    // containers see the wrapper as focusable, auto-focus reaches the list,
    // and Tab cycling can drive into / out of the wrapper correctly.
    fn focusable(&self) -> bool {
        self.list.focusable()
    }

    fn focus_first(&mut self) -> bool {
        self.list.focus_first()
    }

    fn set_focused(&mut self, focused: bool) {
        self.list.set_focused(focused);
    }

    fn layout(&mut self, bounds: Rect) {
        self.bounds = bounds;
        let list_rect = Rect::new(
            bounds.x + 4,
            bounds.y + HEADER_H,
            (bounds.w - 8).max(0),
            (bounds.h - HEADER_H - 4).max(0),
        );
        self.list.layout(list_rect);
    }
}

// ============================================================================
// Directory reading & sorting.
// ============================================================================

struct Entry {
    name: String,
    is_dir: bool,
}

fn read_entries(path: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = read
        .flatten()
        .map(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Entry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir,
            }
        })
        .collect();
    // Directories first, then files; both alphabetical case-insensitive.
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    entries
}
