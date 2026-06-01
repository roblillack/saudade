//! cells — 7GUIs task 7.
//!
//! "Create a simple but usable spreadsheet application. The spreadsheet should
//! be screen-filling. The rows are numbered 0 to 99 and the columns from A to
//! Z. Cells can contain text or formulas. A formula starts with `=`; it can
//! compute over the values of other cells (referenced like `A0`), with the
//! operators `+ - * /` and functions like `SUM` over ranges. Changing a cell's
//! value updates everything that depends on it (change propagation); reference
//! cycles must be detected rather than looping forever."
//!
//! This is the example that builds a real composite widget on top of saudade's
//! primitives: there is no grid or table widget, so [`Grid`] draws the headers
//! and cells itself, hosts two [`ScrollBar`]s for the A–Z / 0–99 viewport, and
//! pops a [`TextInput`] over a cell while it is being edited. Double-click (or
//! Enter, or just start typing) edits the selected cell; Enter commits and steps
//! down, Tab commits and steps right, Escape cancels.
//!
//! The formula engine ([`Sheet`] + the tokenizer / recursive-descent parser
//! below) supports cell references, `+ - * /` with the usual precedence and
//! parentheses, ranges (`A0:A9`) and the functions `SUM`, `PRODUCT`, `AVG`,
//! `MIN`, `MAX`, `COUNT`. Values are recomputed on demand (so edits propagate
//! immediately) and reference cycles are caught by tracking the chain of cells
//! currently being evaluated — a cycle shows as `#CYCLE!` rather than hanging.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use saudade::{
    App, Color, Container, Event, EventCtx, Key, MouseButton, NamedKey, Painter, Point, Rect,
    SCROLLBAR_THICKNESS, ScrollBar, TextInput, Theme, Widget, WindowConfig,
};

const W: i32 = 760;
const H: i32 = 460;
/// The whole grid, including headers and scrollbars, in window coordinates.
const GRID_RECT: Rect = Rect::new(8, 8, W - 16, H - 16);

const COLS: i32 = 26;
const ROWS: i32 = 100;
const COL_W: i32 = 64;
const ROW_H: i32 = 18;
/// Width of the left header column (row numbers) and height of the top header
/// row (column letters).
const HEAD_W: i32 = 34;
const HEAD_H: i32 = 18;

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const GRID_LINE: Color = Color::rgb(0xD0, 0xD0, 0xD0);

fn main() {
    let root = Container::new(W, H).add(Grid::new(seed_sheet()));

    App::new(WindowConfig::new("Cells", W, H), root)
        .with_theme(Theme::windows_31())
        .run();
}

/// A small starter sheet: a two-row "invoice" with a couple of formulas, so the
/// example opens showing live computation rather than a blank grid.
fn seed_sheet() -> Sheet {
    let mut sheet = Sheet::new();
    let rows = [
        ("A0", "Item"),
        ("B0", "Qty"),
        ("C0", "Price"),
        ("D0", "Total"),
        ("A1", "Apples"),
        ("B1", "3"),
        ("C1", "2"),
        ("D1", "=B1*C1"),
        ("A2", "Pears"),
        ("B2", "5"),
        ("C2", "1.5"),
        ("D2", "=B2*C2"),
        ("A3", "Sum"),
        ("D3", "=SUM(D1:D2)"),
        ("A4", "Avg price"),
        ("D4", "=AVG(C1:C2)"),
    ];
    for (cell, content) in rows {
        sheet.set(parse_ref(cell).unwrap(), content.to_string());
    }
    sheet
}

// ============================================================================
// Sheet — raw cell content plus the formula evaluator.
// ============================================================================

/// `(column, row)`, both zero-based: column 0 = `A`, row 0 = the first row.
type CellRef = (u8, u8);

#[derive(Clone, Copy, PartialEq, Eq)]
enum EvalError {
    /// A reference cycle was detected while evaluating.
    Cycle,
    /// Division by zero.
    DivZero,
    /// Malformed formula, unknown function, or out-of-range reference.
    Bad,
}

impl EvalError {
    fn marker(self) -> &'static str {
        match self {
            EvalError::Cycle => "#CYCLE!",
            EvalError::DivZero => "#DIV/0!",
            EvalError::Bad => "#ERROR!",
        }
    }
}

struct Sheet {
    /// Raw user content per cell. Empty cells are simply absent.
    content: HashMap<CellRef, String>,
}

impl Sheet {
    fn new() -> Self {
        Self {
            content: HashMap::new(),
        }
    }

    fn raw(&self, cell: CellRef) -> &str {
        self.content.get(&cell).map(String::as_str).unwrap_or("")
    }

    /// Store (or, for empty input, clear) a cell's content.
    fn set(&mut self, cell: CellRef, text: String) {
        if text.trim().is_empty() {
            self.content.remove(&cell);
        } else {
            self.content.insert(cell, text);
        }
    }

    /// The text shown in the grid: formulas and references compute to a number
    /// (or an error marker); everything else is shown verbatim.
    fn display(&self, cell: CellRef) -> String {
        let raw = self.raw(cell).trim();
        if raw.is_empty() {
            String::new()
        } else if raw.starts_with('=') {
            match self.value_of(cell) {
                Ok(n) => format_number(n),
                Err(e) => e.marker().to_string(),
            }
        } else {
            raw.to_string()
        }
    }

    /// Numeric value of a cell, setting up a fresh evaluation context.
    fn value_of(&self, cell: CellRef) -> Result<f64, EvalError> {
        let mut path = Vec::new();
        let mut memo = HashMap::new();
        self.value(cell, &mut path, &mut memo)
    }

    /// Numeric value of `cell`. `path` is the chain of formula cells currently
    /// being evaluated (for cycle detection); `memo` caches finished results
    /// within one top-level evaluation.
    fn value(
        &self,
        cell: CellRef,
        path: &mut Vec<CellRef>,
        memo: &mut HashMap<CellRef, Result<f64, EvalError>>,
    ) -> Result<f64, EvalError> {
        if let Some(cached) = memo.get(&cell) {
            return *cached;
        }
        let raw = self.raw(cell).trim();
        let result = if raw.is_empty() {
            Ok(0.0)
        } else if let Some(formula) = raw.strip_prefix('=') {
            if path.contains(&cell) {
                // A cycle is path-dependent, so don't cache it.
                return Err(EvalError::Cycle);
            }
            path.push(cell);
            let r = tokenize(formula)
                .and_then(|toks| Parser::new(toks).parse())
                .and_then(|node| self.eval_node(&node, path, memo));
            path.pop();
            r
        } else {
            // A bare number is its value; text counts as 0 in arithmetic.
            Ok(raw.parse::<f64>().unwrap_or(0.0))
        };
        memo.insert(cell, result);
        result
    }

    fn eval_node(
        &self,
        node: &Node,
        path: &mut Vec<CellRef>,
        memo: &mut HashMap<CellRef, Result<f64, EvalError>>,
    ) -> Result<f64, EvalError> {
        match node {
            Node::Num(n) => Ok(*n),
            Node::Ref(c) => self.value(*c, path, memo),
            // A range is only meaningful inside a function's argument list.
            Node::Range(..) => Err(EvalError::Bad),
            Node::Neg(inner) => Ok(-self.eval_node(inner, path, memo)?),
            Node::Bin(op, l, r) => {
                let a = self.eval_node(l, path, memo)?;
                let b = self.eval_node(r, path, memo)?;
                match op {
                    '+' => Ok(a + b),
                    '-' => Ok(a - b),
                    '*' => Ok(a * b),
                    '/' if b == 0.0 => Err(EvalError::DivZero),
                    '/' => Ok(a / b),
                    _ => Err(EvalError::Bad),
                }
            }
            Node::Func(name, args) => {
                let values = self.eval_args(args, path, memo)?;
                apply_func(name, &values)
            }
        }
    }

    /// Flatten a function's arguments into a list of scalar values, expanding
    /// any `A0:B3` range into the values of every cell it covers.
    fn eval_args(
        &self,
        args: &[Node],
        path: &mut Vec<CellRef>,
        memo: &mut HashMap<CellRef, Result<f64, EvalError>>,
    ) -> Result<Vec<f64>, EvalError> {
        let mut out = Vec::new();
        for arg in args {
            match arg {
                Node::Range(a, b) => {
                    for cell in range_cells(*a, *b) {
                        out.push(self.value(cell, path, memo)?);
                    }
                }
                other => out.push(self.eval_node(other, path, memo)?),
            }
        }
        Ok(out)
    }
}

/// Every cell in the rectangular block spanned by two corners (inclusive).
fn range_cells(a: CellRef, b: CellRef) -> Vec<CellRef> {
    let (c0, c1) = (a.0.min(b.0), a.0.max(b.0));
    let (r0, r1) = (a.1.min(b.1), a.1.max(b.1));
    let mut cells = Vec::new();
    for c in c0..=c1 {
        for r in r0..=r1 {
            cells.push((c, r));
        }
    }
    cells
}

fn apply_func(name: &str, values: &[f64]) -> Result<f64, EvalError> {
    match name {
        "SUM" => Ok(values.iter().sum()),
        "PRODUCT" | "PROD" => Ok(values.iter().product()),
        "AVG" | "AVERAGE" | "MEAN" => {
            if values.is_empty() {
                Ok(0.0)
            } else {
                Ok(values.iter().sum::<f64>() / values.len() as f64)
            }
        }
        "MIN" => Ok(values.iter().copied().fold(f64::INFINITY, f64::min))
            .map(|v| if v.is_finite() { v } else { 0.0 }),
        "MAX" => Ok(values.iter().copied().fold(f64::NEG_INFINITY, f64::max))
            .map(|v| if v.is_finite() { v } else { 0.0 }),
        "COUNT" => Ok(values.len() as f64),
        _ => Err(EvalError::Bad),
    }
}

/// Render a number compactly: integers without a decimal point, otherwise up to
/// four trailing-zero-trimmed decimals.
fn format_number(n: f64) -> String {
    if !n.is_finite() {
        return "#NUM!".to_string();
    }
    let rounded = (n * 10_000.0).round() / 10_000.0;
    if (rounded - rounded.round()).abs() < 1e-9 {
        format!("{}", rounded.round() as i64)
    } else {
        let mut s = format!("{rounded:.4}");
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        s
    }
}

// ============================================================================
// Formula parsing — tokenizer + recursive-descent parser into a small AST.
// ============================================================================

#[derive(Clone, PartialEq)]
enum Token {
    Num(f64),
    Cell(CellRef),
    Func(String),
    LParen,
    RParen,
    Comma,
    Colon,
    Op(char),
}

enum Node {
    Num(f64),
    Ref(CellRef),
    Range(CellRef, CellRef),
    Neg(Box<Node>),
    Bin(char, Box<Node>, Box<Node>),
    Func(String, Vec<Node>),
}

/// Parse a label like `"A0"` / `"z99"` into a [`CellRef`]; `None` if malformed
/// or out of the A–Z / 0–99 grid.
fn parse_ref(label: &str) -> Option<CellRef> {
    let toks = tokenize(label).ok()?;
    match toks.as_slice() {
        [Token::Cell(c)] => Some(*c),
        _ => None,
    }
}

fn tokenize(s: &str) -> Result<Vec<Token>, EvalError> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut toks = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let num: String = chars[start..i].iter().collect();
            toks.push(Token::Num(num.parse().map_err(|_| EvalError::Bad)?));
        } else if c.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if i < chars.len() && chars[i] == '(' {
                toks.push(Token::Func(word.to_ascii_uppercase()));
            } else if word.len() == 1 && i < chars.len() && chars[i].is_ascii_digit() {
                let col = word.chars().next().unwrap().to_ascii_uppercase() as i32 - 'A' as i32;
                let dstart = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let row: i32 = chars[dstart..i]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .map_err(|_| EvalError::Bad)?;
                if !(0..COLS).contains(&col) || !(0..ROWS).contains(&row) {
                    return Err(EvalError::Bad);
                }
                toks.push(Token::Cell((col as u8, row as u8)));
            } else {
                return Err(EvalError::Bad);
            }
        } else {
            let tok = match c {
                '+' | '-' | '*' | '/' => Token::Op(c),
                '(' => Token::LParen,
                ')' => Token::RParen,
                ',' => Token::Comma,
                ':' => Token::Colon,
                _ => return Err(EvalError::Bad),
            };
            toks.push(tok);
            i += 1;
        }
    }
    Ok(toks)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(toks: Vec<Token>) -> Self {
        Self { toks, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let tok = self.toks.get(self.pos).cloned();
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn eat(&mut self, expected: &Token) -> Result<(), EvalError> {
        if self.peek() == Some(expected) {
            self.pos += 1;
            Ok(())
        } else {
            Err(EvalError::Bad)
        }
    }

    /// Parse the whole token stream as one expression, rejecting trailing junk.
    fn parse(mut self) -> Result<Node, EvalError> {
        let node = self.parse_expr()?;
        if self.pos == self.toks.len() {
            Ok(node)
        } else {
            Err(EvalError::Bad)
        }
    }

    fn parse_expr(&mut self) -> Result<Node, EvalError> {
        let mut left = self.parse_term()?;
        while let Some(Token::Op(op @ ('+' | '-'))) = self.peek().cloned() {
            self.pos += 1;
            let right = self.parse_term()?;
            left = Node::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Node, EvalError> {
        let mut left = self.parse_factor()?;
        while let Some(Token::Op(op @ ('*' | '/'))) = self.peek().cloned() {
            self.pos += 1;
            let right = self.parse_factor()?;
            left = Node::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_factor(&mut self) -> Result<Node, EvalError> {
        match self.bump() {
            Some(Token::Num(n)) => Ok(Node::Num(n)),
            Some(Token::Op('-')) => Ok(Node::Neg(Box::new(self.parse_factor()?))),
            Some(Token::Op('+')) => self.parse_factor(),
            Some(Token::LParen) => {
                let inner = self.parse_expr()?;
                self.eat(&Token::RParen)?;
                Ok(inner)
            }
            Some(Token::Func(name)) => {
                self.eat(&Token::LParen)?;
                let args = self.parse_args()?;
                self.eat(&Token::RParen)?;
                Ok(Node::Func(name, args))
            }
            Some(Token::Cell(c)) => {
                if self.peek() == Some(&Token::Colon) {
                    self.pos += 1;
                    match self.bump() {
                        Some(Token::Cell(c2)) => Ok(Node::Range(c, c2)),
                        _ => Err(EvalError::Bad),
                    }
                } else {
                    Ok(Node::Ref(c))
                }
            }
            _ => Err(EvalError::Bad),
        }
    }

    fn parse_args(&mut self) -> Result<Vec<Node>, EvalError> {
        let mut args = Vec::new();
        if self.peek() == Some(&Token::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr()?);
            if self.peek() == Some(&Token::Comma) {
                self.pos += 1;
            } else {
                break;
            }
        }
        Ok(args)
    }
}

// ============================================================================
// Grid — the scrollable, editable view of the sheet.
// ============================================================================

// Geometry is derived from the fixed `GRID_RECT`, so these are plain helpers.
fn corner_rect() -> Rect {
    Rect::new(GRID_RECT.x, GRID_RECT.y, HEAD_W, HEAD_H)
}
fn vbar_rect() -> Rect {
    Rect::new(
        GRID_RECT.right() - SCROLLBAR_THICKNESS,
        GRID_RECT.y,
        SCROLLBAR_THICKNESS,
        GRID_RECT.h - SCROLLBAR_THICKNESS,
    )
}
fn hbar_rect() -> Rect {
    Rect::new(
        GRID_RECT.x,
        GRID_RECT.bottom() - SCROLLBAR_THICKNESS,
        GRID_RECT.w - SCROLLBAR_THICKNESS,
        SCROLLBAR_THICKNESS,
    )
}
fn viewport_rect() -> Rect {
    Rect::new(
        GRID_RECT.x + HEAD_W,
        GRID_RECT.y + HEAD_H,
        GRID_RECT.w - HEAD_W - SCROLLBAR_THICKNESS,
        GRID_RECT.h - HEAD_H - SCROLLBAR_THICKNESS,
    )
}
fn visible_cols() -> i32 {
    (viewport_rect().w / COL_W).max(1)
}
fn visible_rows() -> i32 {
    (viewport_rect().h / ROW_H).max(1)
}

fn col_label(col: u8) -> String {
    ((b'A' + col) as char).to_string()
}

struct Grid {
    sheet: Sheet,
    sel: CellRef,
    editing: Option<CellRef>,
    input: TextInput,
    vbar: ScrollBar, // scrolls rows
    hbar: ScrollBar, // scrolls columns
    focused: bool,
    last_click: Option<(CellRef, Instant)>,
}

impl Grid {
    fn new(sheet: Sheet) -> Self {
        let mut vbar = ScrollBar::vertical(vbar_rect());
        let mut hbar = ScrollBar::horizontal(hbar_rect());
        vbar.set_range(visible_rows(), (ROWS - visible_rows()).max(0));
        hbar.set_range(visible_cols(), (COLS - visible_cols()).max(0));
        Self {
            sheet,
            sel: (0, 0),
            editing: None,
            input: TextInput::new(Rect::new(0, 0, COL_W, ROW_H)),
            vbar,
            hbar,
            focused: false,
            last_click: None,
        }
    }

    fn sync_scroll_ranges(&mut self) {
        self.hbar
            .set_range(visible_cols(), (COLS - visible_cols()).max(0));
        self.vbar
            .set_range(visible_rows(), (ROWS - visible_rows()).max(0));
    }

    /// On-screen rect of a cell, or `None` when it is scrolled out of view.
    fn cell_rect(&self, cell: CellRef) -> Option<Rect> {
        let vp = viewport_rect();
        let dc = cell.0 as i32 - self.hbar.value();
        let dr = cell.1 as i32 - self.vbar.value();
        if dc < 0 || dc >= visible_cols() || dr < 0 || dr >= visible_rows() {
            return None;
        }
        Some(Rect::new(
            vp.x + dc * COL_W,
            vp.y + dr * ROW_H,
            COL_W,
            ROW_H,
        ))
    }

    fn cell_at(&self, pos: Point) -> Option<CellRef> {
        let vp = viewport_rect();
        if !vp.contains(pos) {
            return None;
        }
        let col = self.hbar.value() + (pos.x - vp.x) / COL_W;
        let row = self.vbar.value() + (pos.y - vp.y) / ROW_H;
        if (0..COLS).contains(&col) && (0..ROWS).contains(&row) {
            Some((col as u8, row as u8))
        } else {
            None
        }
    }

    fn move_sel(&mut self, dc: i32, dr: i32) {
        self.sel = (
            (self.sel.0 as i32 + dc).clamp(0, COLS - 1) as u8,
            (self.sel.1 as i32 + dr).clamp(0, ROWS - 1) as u8,
        );
        self.ensure_visible();
    }

    fn ensure_visible(&mut self) {
        self.sync_scroll_ranges();
        let (c, r) = (self.sel.0 as i32, self.sel.1 as i32);
        let (left, top) = (self.hbar.value(), self.vbar.value());
        if c < left {
            self.hbar.set_value(c);
        } else if c >= left + visible_cols() {
            self.hbar.set_value(c - visible_cols() + 1);
        }
        if r < top {
            self.vbar.set_value(r);
        } else if r >= top + visible_rows() {
            self.vbar.set_value(r - visible_rows() + 1);
        }
    }

    fn start_edit(&mut self, cell: CellRef, initial: Option<String>) {
        self.sel = cell;
        self.ensure_visible();
        let text = initial.unwrap_or_else(|| self.sheet.raw(cell).to_string());
        self.input.set_text(&text);
        self.input.set_focused(true);
        if let Some(rect) = self.cell_rect(cell) {
            self.input.layout(rect);
        }
        self.editing = Some(cell);
    }

    fn commit_edit(&mut self) {
        if let Some(cell) = self.editing.take() {
            self.sheet.set(cell, self.input.text());
            self.input.set_focused(false);
        }
    }

    fn event_editing(&mut self, event: &Event, ctx: &mut EventCtx) {
        if let Some(cell) = self.editing
            && let Some(rect) = self.cell_rect(cell)
        {
            self.input.layout(rect);
        }
        match event {
            Event::KeyDown {
                key: Key::Named(NamedKey::Enter),
                ..
            } => {
                self.commit_edit();
                self.move_sel(0, 1);
                ctx.request_paint();
            }
            Event::KeyDown {
                key: Key::Named(NamedKey::Escape),
                ..
            } => {
                self.editing = None;
                self.input.set_focused(false);
                ctx.request_paint();
            }
            Event::KeyDown {
                key: Key::Named(NamedKey::Tab),
                ..
            } => {
                self.commit_edit();
                self.move_sel(1, 0);
                ctx.consume_event();
                ctx.request_paint();
            }
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } => {
                if self.input.bounds().contains(*pos) {
                    self.input.event(event, ctx);
                } else {
                    self.commit_edit();
                    if let Some(cell) = self.cell_at(*pos) {
                        self.sel = cell;
                    }
                    ctx.request_paint();
                }
            }
            // Everything else (text, Backspace, cursor moves, blink Tick, drag)
            // belongs to the embedded input.
            _ => self.input.event(event, ctx),
        }
    }

    fn event_grid(&mut self, event: &Event, ctx: &mut EventCtx) {
        match event {
            Event::PointerDown {
                pos,
                button: MouseButton::Left,
            } => {
                ctx.request_focus();
                if let Some(cell) = self.cell_at(*pos) {
                    let now = Instant::now();
                    let double = self
                        .last_click
                        .is_some_and(|(c, t)| c == cell && now.duration_since(t) <= DOUBLE_CLICK);
                    self.sel = cell;
                    if double {
                        self.last_click = None;
                        self.start_edit(cell, None);
                    } else {
                        self.last_click = Some((cell, now));
                    }
                }
                ctx.request_paint();
            }
            Event::KeyDown {
                key: Key::Named(named),
                modifiers,
            } if self.focused && !modifiers.has_command() => {
                match named {
                    NamedKey::Up => self.move_sel(0, -1),
                    NamedKey::Down => self.move_sel(0, 1),
                    NamedKey::Left => self.move_sel(-1, 0),
                    NamedKey::Right => self.move_sel(1, 0),
                    NamedKey::Enter => self.start_edit(self.sel, None),
                    NamedKey::Backspace | NamedKey::Delete => {
                        self.sheet.set(self.sel, String::new());
                    }
                    _ => {}
                }
                ctx.request_paint();
            }
            // Start editing as soon as the user types a printable character.
            Event::Char { ch, modifiers }
                if self.focused && !modifiers.has_command() && *ch >= ' ' =>
            {
                self.start_edit(self.sel, Some(ch.to_string()));
                ctx.request_paint();
            }
            _ => {}
        }
    }
}

impl Widget for Grid {
    fn bounds(&self) -> Rect {
        GRID_RECT
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.sync_scroll_ranges();
        let (left, top) = (self.hbar.value(), self.vbar.value());
        let vp = viewport_rect();

        painter.fill_rect(GRID_RECT, theme.face);
        painter.button(corner_rect(), theme, false, false);

        // Column letters across the top, row numbers down the left.
        for vc in 0..visible_cols() {
            let col = left + vc;
            if col >= COLS {
                break;
            }
            let rect = Rect::new(vp.x + vc * COL_W, GRID_RECT.y, COL_W, HEAD_H);
            draw_header(
                painter,
                theme,
                rect,
                &col_label(col as u8),
                col == self.sel.0 as i32,
            );
        }
        for vr in 0..visible_rows() {
            let row = top + vr;
            if row >= ROWS {
                break;
            }
            let rect = Rect::new(GRID_RECT.x, vp.y + vr * ROW_H, HEAD_W, ROW_H);
            draw_header(
                painter,
                theme,
                rect,
                &row.to_string(),
                row == self.sel.1 as i32,
            );
        }

        // Cells, clipped to the viewport.
        let saved = painter.push_clip(vp);
        for vr in 0..visible_rows() {
            let row = top + vr;
            if row >= ROWS {
                break;
            }
            for vc in 0..visible_cols() {
                let col = left + vc;
                if col >= COLS {
                    break;
                }
                let cell = (col as u8, row as u8);
                let rect = Rect::new(vp.x + vc * COL_W, vp.y + vr * ROW_H, COL_W, ROW_H);
                painter.fill_rect(rect, Color::WHITE);
                painter.h_line(rect.x, rect.bottom() - 1, rect.w, GRID_LINE);
                painter.v_line(rect.right() - 1, rect.y, rect.h, GRID_LINE);

                if self.editing != Some(cell) {
                    let text = self.sheet.display(cell);
                    if !text.is_empty() {
                        let ty = rect.y + (rect.h - theme.font_size as i32) / 2;
                        let clip = painter.push_clip(rect.inset(1));
                        painter.text(rect.x + 3, ty, &text, theme.font_size, theme.text);
                        painter.restore_clip(clip);
                    }
                }

                if cell == self.sel && self.editing.is_none() {
                    painter.stroke_rect(rect, theme.highlight_bg);
                    painter.stroke_rect(rect.inset(1), theme.highlight_bg);
                }
            }
        }
        painter.restore_clip(saved);

        // The editor sits on top of its cell.
        if let Some(cell) = self.editing
            && let Some(rect) = self.cell_rect(cell)
        {
            self.input.layout(rect);
            self.input.paint(painter, theme);
        }

        self.vbar.paint(painter, theme);
        self.hbar.paint(painter, theme);
        painter.stroke_rect(GRID_RECT, theme.border);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        // The scrollbars get first refusal on pointer events in their lanes.
        if self.vbar.captures_pointer() {
            self.vbar.event(event, ctx);
            ctx.request_paint();
            return;
        }
        if self.hbar.captures_pointer() {
            self.hbar.event(event, ctx);
            ctx.request_paint();
            return;
        }
        if let Some(pos) = event.position() {
            if vbar_rect().contains(pos) {
                self.vbar.event(event, ctx);
                ctx.request_paint();
                return;
            }
            if hbar_rect().contains(pos) {
                self.hbar.event(event, ctx);
                ctx.request_paint();
                return;
            }
        }

        if self.editing.is_some() {
            self.event_editing(event, ctx);
        } else {
            self.event_grid(event, ctx);
        }
    }

    fn captures_pointer(&self) -> bool {
        self.vbar.captures_pointer() || self.hbar.captures_pointer()
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.commit_edit();
        }
    }

    fn wants_ticks(&self) -> bool {
        // Drive the editor's caret blink while editing.
        self.editing.is_some()
    }
}

/// A raised header cell with a centered label; selected headers invert to the
/// navy/white highlight so the current cell's coordinates stand out.
fn draw_header(painter: &mut Painter, theme: &Theme, rect: Rect, label: &str, selected: bool) {
    painter.button(rect, theme, false, false);
    let fg = if selected {
        painter.fill_rect(rect.inset(1), theme.highlight_bg);
        theme.highlight_text
    } else {
        theme.text
    };
    painter.text_centered(rect, label, theme.font_size, fg);
}

// ============================================================================
// Tests — the formula engine is pure, so it runs headless and documents the
// supported syntax and the change-propagation / cycle rules.
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(cells: &[(&str, &str)]) -> Sheet {
        let mut s = Sheet::new();
        for (r, c) in cells {
            s.set(parse_ref(r).unwrap(), c.to_string());
        }
        s
    }

    fn display(s: &Sheet, cell: &str) -> String {
        s.display(parse_ref(cell).unwrap())
    }

    #[test]
    fn parses_cell_references() {
        assert_eq!(parse_ref("A0"), Some((0, 0)));
        assert_eq!(parse_ref("Z99"), Some((25, 99)));
        assert_eq!(parse_ref("b12"), Some((1, 12))); // lower-case accepted
        assert_eq!(parse_ref("A100"), None); // row out of range
        assert_eq!(parse_ref("AA1"), None); // two-letter column
    }

    #[test]
    fn literals_and_text_display_verbatim() {
        let s = sheet(&[("A0", "Hello"), ("B0", "42"), ("C0", "3.5")]);
        assert_eq!(display(&s, "A0"), "Hello");
        assert_eq!(display(&s, "B0"), "42");
        assert_eq!(display(&s, "C0"), "3.5");
        assert_eq!(display(&s, "Z9"), ""); // empty
    }

    #[test]
    fn arithmetic_respects_precedence_and_parentheses() {
        let s = sheet(&[
            ("A0", "=1+2*3"),
            ("A1", "=(1+2)*3"),
            ("A2", "=10/4"),
            ("A3", "=-2 + 5"),
        ]);
        assert_eq!(display(&s, "A0"), "7");
        assert_eq!(display(&s, "A1"), "9");
        assert_eq!(display(&s, "A2"), "2.5");
        assert_eq!(display(&s, "A3"), "3");
    }

    #[test]
    fn references_and_change_propagation() {
        let mut s = sheet(&[("A0", "2"), ("A1", "3"), ("B0", "=A0+A1"), ("C0", "=B0*2")]);
        assert_eq!(display(&s, "B0"), "5");
        assert_eq!(display(&s, "C0"), "10");
        // Editing A0 propagates through B0 into C0.
        s.set((0, 0), "10".to_string());
        assert_eq!(display(&s, "B0"), "13");
        assert_eq!(display(&s, "C0"), "26");
    }

    #[test]
    fn functions_over_ranges() {
        let s = sheet(&[
            ("A0", "1"),
            ("A1", "2"),
            ("A2", "3"),
            ("A3", "4"),
            ("B0", "=SUM(A0:A3)"),
            ("B1", "=AVG(A0:A3)"),
            ("B2", "=MIN(A0:A3)"),
            ("B3", "=MAX(A0:A3)"),
            ("B4", "=COUNT(A0:A3)"),
            ("B5", "=SUM(A0:A3) + 10"),
        ]);
        assert_eq!(display(&s, "B0"), "10");
        assert_eq!(display(&s, "B1"), "2.5");
        assert_eq!(display(&s, "B2"), "1");
        assert_eq!(display(&s, "B3"), "4");
        assert_eq!(display(&s, "B4"), "4");
        assert_eq!(display(&s, "B5"), "20");
    }

    #[test]
    fn errors_are_reported_not_fatal() {
        let s = sheet(&[
            ("A0", "=1/0"),
            ("A1", "=1+"),
            ("A2", "=NOPE(1)"),
            ("A3", "=A0"), // inherits the upstream error
        ]);
        assert_eq!(display(&s, "A0"), "#DIV/0!");
        assert_eq!(display(&s, "A1"), "#ERROR!");
        assert_eq!(display(&s, "A2"), "#ERROR!");
        assert_eq!(display(&s, "A3"), "#DIV/0!");
    }

    #[test]
    fn reference_cycles_are_detected() {
        let direct = sheet(&[("A0", "=A0")]);
        assert_eq!(display(&direct, "A0"), "#CYCLE!");

        let indirect = sheet(&[("A0", "=A1"), ("A1", "=A2"), ("A2", "=A0")]);
        assert_eq!(display(&indirect, "A0"), "#CYCLE!");
        assert_eq!(display(&indirect, "A1"), "#CYCLE!");
    }

    #[test]
    fn diamond_dependencies_are_not_cycles() {
        // A0 feeds both B0 and B1, which both feed C0 — a DAG, not a cycle.
        let s = sheet(&[
            ("A0", "5"),
            ("B0", "=A0+1"),
            ("B1", "=A0+2"),
            ("C0", "=B0+B1"),
        ]);
        assert_eq!(display(&s, "C0"), "13");
    }

    // Drive the grid through the mock backend: render the seed sheet, edit an
    // empty cell into a formula, and scroll to the bottom edge. This catches
    // paint-time panics (clip-stack balance, evaluating formulas during paint,
    // the row/column end-of-grid break) and event-routing bugs that the pure
    // engine tests can't see.
    #[test]
    fn grid_edits_and_scrolls_without_panicking() {
        use saudade::Modifiers;
        use saudade::mock::MockBackend;

        let mut grid = Grid::new(seed_sheet());
        grid.set_focused(true);
        let backend = MockBackend::new(W, H);
        backend.render(&mut grid);

        // Double-click the empty cell F5 to start editing it.
        let click = Event::PointerDown {
            pos: Point::new(366, 120),
            button: MouseButton::Left,
        };
        backend.dispatch(&mut grid, &click);
        backend.dispatch(&mut grid, &click);
        backend.render(&mut grid);

        // Type "=6*7" and commit with Enter.
        for ch in "=6*7".chars() {
            backend.dispatch(
                &mut grid,
                &Event::Char {
                    ch,
                    modifiers: Modifiers::default(),
                },
            );
        }
        backend.dispatch(
            &mut grid,
            &Event::KeyDown {
                key: Key::Named(NamedKey::Enter),
                modifiers: Modifiers::default(),
            },
        );
        assert_eq!(grid.sheet.display((5, 5)), "42");
        backend.render(&mut grid);

        // Drag the vertical scrollbar thumb to the bottom and repaint the
        // last rows (exercises the `row >= ROWS` break).
        backend.dispatch(
            &mut grid,
            &Event::PointerDown {
                pos: Point::new(744, 30),
                button: MouseButton::Left,
            },
        );
        backend.dispatch(
            &mut grid,
            &Event::PointerMove {
                pos: Point::new(744, 430),
            },
        );
        backend.dispatch(
            &mut grid,
            &Event::PointerUp {
                pos: Point::new(744, 430),
                button: MouseButton::Left,
            },
        );
        backend.render(&mut grid);
        assert_eq!(grid.sheet.display((5, 5)), "42");
    }
}
