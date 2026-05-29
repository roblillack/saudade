//! filer — a tiny retrogui demo that walks the local filesystem inside a
//! list widget. Double-click (or Enter) on a directory descends into it;
//! double-click on `..` ascends to the parent.

use std::path::{Path, PathBuf};

use retrogui::{
    App, Color, Event, EventCtx, List, ListIcon, ListItem, Painter, Rect, Theme, Widget,
    WindowConfig,
};

const WINDOW_W: i32 = 420;
const WINDOW_H: i32 = 360;
const HEADER_H: i32 = 22;

fn main() {
    let start = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"));
    let start = start
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("/"));

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
    icons: Icons,
}

impl FileBrowser {
    fn new(path: PathBuf) -> Self {
        let mut me = Self {
            list: List::new(Rect::new(0, 0, 0, 0)),
            path,
            bounds: Rect::new(0, 0, 0, 0),
            icons: Icons::new(),
        };
        me.reload();
        me
    }

    fn reload(&mut self) {
        let entries = read_entries(&self.path);
        let mut items = Vec::with_capacity(entries.len() + 1);
        if self.path.parent().is_some() {
            items.push(ListItem::new("..").with_icon(self.icons.up.clone()));
        }
        for entry in entries {
            let icon = if entry.is_dir {
                self.icons.folder.clone()
            } else {
                self.icons.file.clone()
            };
            items.push(ListItem::new(entry.name).with_icon(icon));
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
        let Some(idx) = self.list.take_activated() else { return };
        let name = self.list.items().get(idx).map(|i| i.label.clone());
        if let Some(name) = name {
            self.descend(&name);
            ctx.request_paint();
        }
    }
}

impl Widget for FileBrowser {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        // Background tint for the whole window — the list paints itself
        // sunken-white over the top.
        painter.fill_rect(self.bounds, theme.face);
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
        self.list.event(event, ctx);
        self.handle_activation(ctx);
    }

    fn captures_pointer(&self) -> bool {
        self.list.captures_pointer()
    }

    fn focusable(&self) -> bool {
        true
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

// ============================================================================
// Icons — tiny 16x16 procedural glyphs for folder / file / up-arrow.
// ============================================================================

struct Icons {
    folder: ListIcon,
    file: ListIcon,
    up: ListIcon,
}

impl Icons {
    fn new() -> Self {
        Self {
            folder: folder_icon(),
            file: file_icon(),
            up: up_icon(),
        }
    }
}

fn folder_icon() -> ListIcon {
    let mut icon = ListIcon::new(16, 16);
    let line = Color::BLACK;
    let body = Color::YELLOW;
    // Tab on the top-left of the folder.
    icon.fill_rect(Rect::new(1, 3, 6, 1), line);
    icon.fill_rect(Rect::new(1, 4, 1, 2), line);
    icon.fill_rect(Rect::new(6, 4, 1, 1), line);
    // Top edge slants out to meet the wider body.
    icon.fill_rect(Rect::new(7, 5, 7, 1), line);
    // Main body outline.
    icon.fill_rect(Rect::new(1, 6, 13, 1), line);
    icon.fill_rect(Rect::new(1, 6, 1, 8), line);
    icon.fill_rect(Rect::new(13, 6, 1, 8), line);
    icon.fill_rect(Rect::new(1, 13, 13, 1), line);
    // Yellow fill inside both shapes.
    icon.fill_rect(Rect::new(2, 4, 4, 2), body);
    icon.fill_rect(Rect::new(2, 7, 11, 6), body);
    icon
}

fn file_icon() -> ListIcon {
    let mut icon = ListIcon::new(16, 16);
    let line = Color::BLACK;
    let body = Color::WHITE;
    // Page outline: rectangle with a folded top-right corner.
    icon.fill_rect(Rect::new(3, 1, 7, 1), line); // top edge
    icon.fill_rect(Rect::new(3, 1, 1, 13), line); // left edge
    icon.fill_rect(Rect::new(3, 13, 9, 1), line); // bottom edge
    icon.fill_rect(Rect::new(11, 5, 1, 9), line); // right edge below fold
    // Diagonal corner — the dog-ear.
    icon.set_pixel(10, 1, line);
    icon.set_pixel(10, 2, line);
    icon.set_pixel(11, 2, line);
    icon.set_pixel(11, 3, line);
    icon.set_pixel(12, 3, line);
    icon.set_pixel(12, 4, line);
    icon.set_pixel(11, 4, line);
    // Fold underside.
    icon.fill_rect(Rect::new(9, 4, 3, 1), line);
    // White fill.
    icon.fill_rect(Rect::new(4, 2, 6, 3), body);
    icon.fill_rect(Rect::new(4, 5, 7, 8), body);
    // Page lines.
    icon.fill_rect(Rect::new(5, 7, 5, 1), line);
    icon.fill_rect(Rect::new(5, 9, 5, 1), line);
    icon.fill_rect(Rect::new(5, 11, 4, 1), line);
    icon
}

fn up_icon() -> ListIcon {
    let mut icon = ListIcon::new(16, 16);
    let line = Color::BLACK;
    // Arrow head — five rows of an upward triangle.
    for y in 0..5 {
        let half = y + 1;
        let cx = 7;
        let xs = cx - half + 1;
        let xe = cx + half;
        icon.fill_rect(Rect::new(xs, 3 + y, xe - xs + 1, 1), line);
    }
    // Shaft.
    icon.fill_rect(Rect::new(6, 8, 4, 5), line);
    icon
}
