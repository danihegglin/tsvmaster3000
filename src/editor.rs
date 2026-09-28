//! Editor state and rendering.
//!
//! The whole window is painted by one `canvas`: every frame only the visible
//! cells are read from the table, truncated to their column width and shaped
//! (gpui caches shaped lines across frames), so cost is independent of the
//! file size.

use std::path::PathBuf;

use gpui::{
    App, Bounds, ClipboardItem, Context, FocusHandle, Font, FontWeight, Hsla, KeyDownEvent,
    Keystroke, MouseButton, MouseDownEvent, PaintQuad, Pixels, Render, ScrollWheelEvent,
    SharedString, TextRun, Window, canvas, div, fill, font, point, prelude::*, px, rgb, size,
    Background, BorderStyle, linear_color_stop, linear_gradient, quad, transparent_black,
};
use regex::Regex;

use crate::lineedit::LineEdit;
use crate::table::{Change, Row, Table};

// Tokyo Night pushed icier: bluer backgrounds, and its cyans and teals in
// place of the warm accents. Accent names follow tokyonight.nvim.
const BG: u32 = 0x161a2c;
const BG_DARK: u32 = 0x11141f;
const BG_STRIPE: u32 = 0x191e33;
const BG_HIGHLIGHT: u32 = 0x21283f;
const HEADER_TOP: u32 = 0x1d2340;
const HEADER_BOTTOM: u32 = 0x171c30;
const BAR_TOP: u32 = 0x151928;
const BLACK: u32 = 0x0d0f18;
const FG: u32 = 0xc8d3f5;
const FG_DARK: u32 = 0x9aa8d6;
const FG_GUTTER: u32 = 0x363f63;
const COMMENT: u32 = 0x56608f;
const GRID: u32 = 0x1f2539;
const BLUE: u32 = 0x7aa2f7;
const BLUE1: u32 = 0x2ac3de;
const BLUE5: u32 = 0x89ddff;
const ICE: u32 = 0xb4f9f8;
const TEAL: u32 = 0x73daca;
const MAGENTA: u32 = 0xbb9af7;
const RED: u32 = 0xf7768e;

const HEADER_FG: u32 = BLUE5;
const NUM_FG: u32 = TEAL;
const NORMAL_C: u32 = BLUE;
const INSERT_C: u32 = TEAL;
const VISUAL_C: u32 = MAGENTA;
const COMMAND_C: u32 = BLUE1;
const FILTER_C: u32 = ICE;

pub const DEFAULT_FONT_SIZE: f32 = 13.0;
const MIN_WIDTH: u16 = 3;
const MAX_AUTO_WIDTH: u16 = 40;
const PAD: f32 = 6.0;

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Normal,
    Insert,
    Visual { line: bool },
    Command,
    Search { forward: bool },
    Filter,
}

pub enum Register {
    Empty,
    Rows(Vec<Vec<String>>),
    Block(Vec<Vec<String>>),
}

pub struct Group {
    pub changes: Vec<Change>,
    pub cursor: (usize, usize),
}

#[derive(Default)]
pub struct History {
    pub undo: Vec<Group>,
    pub redo: Vec<Group>,
    pub cur: Vec<Change>,
    pub cur_cursor: (usize, usize),
    pub saved_depth: usize,
}

pub struct Search {
    pub re: Regex,
    pub pattern: String,
    pub forward: bool,
}

#[derive(Default)]
pub struct Dot {
    pub rec: Vec<String>,
    pub last: Vec<String>,
    pub in_insert: bool,
    pub replaying: bool,
}

pub struct Message {
    pub text: String,
    pub error: bool,
}

#[derive(Clone, Copy)]
struct Metrics {
    font_size: f32,
    char_w: f32,
    row_h: f32,
}

#[derive(Default)]
pub struct View {
    pub width: f32,
    pub height: f32,
    pub gutter_w: f32,
    pub data_y: f32,
    pub rows: usize,
    /// Visible columns: (index, x0, x1).
    pub cols: Vec<(usize, f32, f32)>,
    /// The "+" buttons: x range in the column bar, y of the gutter cell.
    pub add_col: Option<(f32, f32)>,
    pub add_row: Option<f32>,
}

pub struct Editor {
    pub focus: FocusHandle,
    pub table: Table,
    pub path: Option<PathBuf>,
    pub row: usize,
    pub col: usize,
    pub anchor: (usize, usize),
    pub top: usize,
    pub left: usize,
    pub follow: bool,
    pub mode: Mode,
    pub pending: Vec<String>,
    pub history: History,
    pub register: Register,
    pub pending_register: Option<Register>,
    pub clipboard_last: Option<String>,
    pub edit: LineEdit,
    pub cmdline: LineEdit,
    pub cmd_history: Vec<String>,
    pub cmd_history_pos: Option<usize>,
    pub message: Option<Message>,
    pub search: Option<Search>,
    pub highlight: bool,
    /// Fuzzy filter query; cells that don't match are drawn empty.
    pub filter: String,
    /// While filtering: the table rows still shown, ascending (the pinned
    /// header always is). `None` shows every row. Row motions, scrolling and
    /// row-range edits work on these rows only.
    pub shown: Option<Vec<usize>>,
    pub widths: Vec<u16>,
    pub header: bool,
    pub relative: bool,
    pub font_size: f32,
    pub dot: Dot,
    /// Set when the last normal-mode command was a lone `x` (true if it
    /// changed the cell), so a second `x` can turn it into a column delete.
    pub after_x: Option<bool>,
    pub view: View,
    scroll_acc: (f32, f32),
    metrics: Option<Metrics>,
    title: String,
}

pub fn col_name(mut c: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

pub fn display_width(s: &str) -> usize {
    if s.is_ascii() {
        s.len()
    } else {
        s.chars().count()
    }
}

/// fzf-style fuzzy match: every whitespace-separated term of `query` must
/// appear in `s` as a subsequence. Smartcase, like search.
pub fn fuzzy_match(query: &str, s: &str) -> bool {
    let icase = !query.chars().any(char::is_uppercase);
    query.split_whitespace().all(|term| {
        let mut want = term.chars().peekable();
        for ch in s.chars() {
            let Some(&w) = want.peek() else { break };
            if ch == w || (icase && ch.to_lowercase().eq(w.to_lowercase())) {
                want.next();
            }
        }
        want.peek().is_none()
    })
}

fn looks_numeric(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.as_bytes()[s.len() - 1].is_ascii_digit()
        && s.parse::<f64>().is_ok()
}

/// Truncate to at most `max` chars, ending in an ellipsis when cut.
fn truncate(s: &str, max: usize) -> SharedString {
    if max == 0 {
        return SharedString::default();
    }
    match s.char_indices().nth(max) {
        None => SharedString::from(s.to_owned()),
        Some(_) => {
            let cut = s.char_indices().nth(max - 1).map_or(s.len(), |(i, _)| i);
            SharedString::from(format!("{}…", &s[..cut]))
        }
    }
}

impl Editor {
    pub fn new(
        table: Table,
        path: Option<PathBuf>,
        message: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut editor = Self {
            focus: cx.focus_handle(),
            table,
            path,
            row: 0,
            col: 0,
            anchor: (0, 0),
            top: 0,
            left: 0,
            follow: true,
            mode: Mode::Normal,
            pending: Vec::new(),
            history: History::default(),
            register: Register::Empty,
            pending_register: None,
            clipboard_last: None,
            edit: LineEdit::default(),
            cmdline: LineEdit::default(),
            cmd_history: Vec::new(),
            cmd_history_pos: None,
            message: message.map(|text| Message { text, error: false }),
            search: None,
            highlight: false,
            filter: String::new(),
            shown: None,
            widths: Vec::new(),
            header: true,
            relative: false,
            font_size: DEFAULT_FONT_SIZE,
            dot: Dot::default(),
            after_x: None,
            view: View::default(),
            scroll_acc: (0.0, 0.0),
            metrics: None,
            title: String::new(),
        };
        editor.fit_all();
        editor
    }

    pub fn replace_table(&mut self, table: Table, path: Option<PathBuf>) {
        self.table = table;
        self.path = path;
        self.row = 0;
        self.col = 0;
        self.top = 0;
        self.left = 0;
        self.mode = Mode::Normal;
        self.history = History::default();
        self.filter.clear();
        self.shown = None;
        self.follow = true;
        self.fit_all();
    }

    // ---- basic state helpers ---------------------------------------------

    pub fn set_msg(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            error: false,
        });
    }

    pub fn set_err(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            error: true,
        });
    }

    pub fn is_dirty(&self) -> bool {
        !self.history.cur.is_empty() || self.history.undo.len() != self.history.saved_depth
    }

    pub fn header_active(&self) -> bool {
        self.header && self.table.nrows() > 1
    }

    pub fn first_data_row(&self) -> usize {
        self.header_active() as usize
    }

    pub fn last_row(&self) -> usize {
        self.table.nrows() - 1
    }

    pub fn last_col(&self) -> usize {
        self.table.ncols() - 1
    }

    pub fn clamp_cursor(&mut self) {
        self.row = self.row.min(self.last_row());
        self.col = self.col.min(self.last_col());
        self.anchor.0 = self.anchor.0.min(self.last_row());
        self.anchor.1 = self.anchor.1.min(self.last_col());
    }

    pub fn page_rows(&self) -> usize {
        self.view.rows.max(1)
    }

    pub fn cell(&self) -> &str {
        self.table.cell(self.row, self.col)
    }

    /// (r0, r1, c0, c1), inclusive.
    pub fn selection(&self) -> (usize, usize, usize, usize) {
        let (r0, r1) = (self.anchor.0.min(self.row), self.anchor.0.max(self.row));
        match self.mode {
            Mode::Visual { line: true } => (r0, r1, 0, self.last_col()),
            _ => (
                r0,
                r1,
                self.anchor.1.min(self.col),
                self.anchor.1.max(self.col),
            ),
        }
    }

    // ---- filtered rows -----------------------------------------------------
    //
    // Rows are addressed by table index everywhere; "display" indices count
    // shown rows only and are what motions and scrolling step through.

    pub fn disp_len(&self) -> usize {
        self.shown.as_ref().map_or(self.table.nrows(), Vec::len)
    }

    pub fn disp_row(&self, d: usize) -> usize {
        match &self.shown {
            Some(rows) => rows[d.min(rows.len() - 1)],
            None => d.min(self.last_row()),
        }
    }

    /// Display index of row `r`, or of the next shown row if it's hidden.
    pub fn disp_of(&self, r: usize) -> usize {
        match &self.shown {
            Some(rows) => rows.partition_point(|&x| x < r).min(rows.len() - 1),
            None => r,
        }
    }

    /// The row `delta` shown rows away from `r`, clamped to the ends.
    pub fn move_rows(&self, r: usize, delta: isize) -> usize {
        let d = self.disp_of(r) as isize + delta;
        self.disp_row(d.clamp(0, self.disp_len() as isize - 1) as usize)
    }

    /// The shown rows in `r0..=r1`.
    pub fn rows_in(&self, r0: usize, r1: usize) -> Vec<usize> {
        match &self.shown {
            Some(rows) => {
                let a = rows.partition_point(|&x| x < r0);
                let b = rows.partition_point(|&x| x <= r1);
                rows[a..b].to_vec()
            }
            None => (r0..=r1).collect(),
        }
    }

    fn row_matches(&self, query: &str, r: usize) -> bool {
        // A cell match implies a match of the tab-joined line: cheap prefilter.
        fuzzy_match(query, &self.table.line(r))
            && self.table.fields(r).any(|c| !c.is_empty() && fuzzy_match(query, c))
    }

    /// Set the filter query and recompute the shown rows. Typing more of the
    /// query only narrows, so then only the rows shown so far are checked.
    pub fn set_filter(&mut self, query: String) {
        if query.trim().is_empty() {
            self.filter = query;
            self.shown = None;
            return;
        }
        let narrowing = !self.filter.trim().is_empty() && query.starts_with(&self.filter);
        let candidates = match self.shown.take() {
            Some(rows) if narrowing => rows,
            _ => (0..self.table.nrows()).collect(),
        };
        let header = self.header_active();
        let rows = candidates
            .into_iter()
            .filter(|&r| (header && r == 0) || self.row_matches(&query, r))
            .collect();
        self.filter = query;
        self.shown = Some(rows);
        self.fix_shown();
    }

    /// Recompute the shown rows from scratch (after the table changed).
    pub fn refilter(&mut self) {
        if self.shown.is_some() {
            self.shown = None;
            self.set_filter(self.filter.clone());
        }
    }

    /// Keep at least one data row shown (an empty result shows the cursor
    /// row, blanked), so there's always somewhere for the cursor to be.
    fn fix_shown(&mut self) {
        let header = self.header_active();
        let nrows = self.table.nrows();
        let fallback = if header { self.row.clamp(1, nrows - 1) } else { self.row.min(nrows - 1) };
        let Some(rows) = &mut self.shown else { return };
        rows.retain(|&r| r < nrows);
        if rows.len() <= header as usize && !rows.contains(&fallback) {
            let at = rows.partition_point(|&x| x < fallback);
            rows.insert(at, fallback);
        }
    }

    /// Apply a change to the table, keeping the shown rows in step when rows
    /// are inserted or removed. Inserted rows are shown.
    fn apply(&mut self, change: Change) -> Change {
        if let (Some(rows), Change::Splice { at, remove, insert }) = (&mut self.shown, &change) {
            let (at, remove, added) = (*at, *remove, insert.len());
            rows.retain(|&r| r < at || r >= at + remove);
            for r in rows.iter_mut().filter(|r| **r >= at + remove) {
                *r = *r + added - remove;
            }
            let pos = rows.partition_point(|&r| r < at);
            rows.splice(pos..pos, at..at + added);
        }
        let inverse = self.table.apply(change);
        self.fix_shown();
        inverse
    }

    // ---- editing primitives ----------------------------------------------

    pub fn record(&mut self, change: Change) {
        if self.history.cur.is_empty() {
            self.history.cur_cursor = (self.row, self.col);
        }
        let inverse = self.apply(change);
        self.history.cur.push(inverse);
    }

    /// Close the current undo group.
    pub fn commit(&mut self) {
        if self.history.cur.is_empty() {
            return;
        }
        let changes = std::mem::take(&mut self.history.cur);
        if self.history.saved_depth > self.history.undo.len() {
            self.history.saved_depth = usize::MAX;
        }
        self.history.undo.push(Group {
            changes,
            cursor: self.history.cur_cursor,
        });
        self.history.redo.clear();
    }

    pub fn undo(&mut self, redo: bool) {
        self.commit();
        let (from, verb) = if redo {
            (&mut self.history.redo, "redo")
        } else {
            (&mut self.history.undo, "undo")
        };
        let Some(group) = from.pop() else {
            self.set_err(if redo {
                "Already at newest change"
            } else {
                "Already at oldest change"
            });
            return;
        };
        let ncols = self.table.ncols();
        let mut inverse = Vec::with_capacity(group.changes.len());
        for change in group.changes.into_iter().rev() {
            inverse.push(self.apply(change));
        }
        let n = inverse.len();
        let to = if redo {
            &mut self.history.undo
        } else {
            &mut self.history.redo
        };
        to.push(Group {
            changes: inverse,
            cursor: group.cursor,
        });
        (self.row, self.col) = group.cursor;
        self.clamp_cursor();
        if self.table.ncols() != ncols {
            self.fit_all();
        }
        self.set_msg(format!("{verb}: {n} change{}", if n == 1 { "" } else { "s" }));
    }

    pub fn set_cell(&mut self, r: usize, c: usize, value: String) {
        if self.table.cell(r, c) == value {
            return;
        }
        if value.is_empty() && c >= self.table.row_len(r) {
            return;
        }
        let w = display_width(&value).min(MAX_AUTO_WIDTH as usize) as u16;
        self.record(Change::Cell {
            row: r,
            col: c,
            value,
            shrink_to: None,
        });
        self.ensure_widths();
        if w > self.widths[c] {
            self.widths[c] = w;
        }
    }

    pub fn clear_block(&mut self, r0: usize, r1: usize, c0: usize, c1: usize) {
        for r in self.rows_in(r0, r1) {
            let len = self.table.row_len(r);
            for c in c0..=c1.min(len.saturating_sub(1)) {
                self.set_cell(r, c, String::new());
            }
        }
    }

    pub fn map_block(
        &mut self,
        (r0, r1, c0, c1): (usize, usize, usize, usize),
        f: impl Fn(&str) -> Option<String>,
    ) {
        for r in self.rows_in(r0, r1) {
            let len = self.table.row_len(r);
            for c in c0..=c1.min(len.saturating_sub(1)) {
                if let Some(v) = f(self.table.cell(r, c)) {
                    self.set_cell(r, c, v);
                }
            }
        }
    }

    /// Delete the (shown) rows in `r0..=r1`.
    pub fn delete_rows(&mut self, r0: usize, r1: usize) {
        let r1 = r1.min(self.last_row());
        let rows = self.rows_in(r0, r1);
        self.yank_rows(r0, r1);
        // Bottom-up, one splice per contiguous run.
        let mut end = rows.len();
        while end > 0 {
            let mut start = end - 1;
            while start > 0 && rows[start - 1] + 1 == rows[start] {
                start -= 1;
            }
            self.record(Change::Splice {
                at: rows[start],
                remove: end - start,
                insert: Vec::new(),
            });
            end = start;
        }
        self.row = r0.min(self.last_row());
        self.set_msg(format!("{} fewer rows", rows.len()));
    }

    pub fn insert_rows(&mut self, at: usize, rows: Vec<Vec<String>>) {
        let ncols = self.table.ncols();
        let insert = rows.into_iter().map(Row::Owned).collect();
        self.record(Change::Splice {
            at,
            remove: 0,
            insert,
        });
        if self.table.ncols() != ncols {
            self.ensure_widths();
        }
    }

    pub fn insert_col(&mut self, at: usize) {
        self.record(Change::InsertCol { at, values: None });
        self.widths.insert(at.min(self.widths.len()), 8);
        self.ensure_widths();
    }

    pub fn delete_cols(&mut self, at: usize, n: usize) {
        let n = n.min(self.table.ncols() - at);
        if self.table.ncols() <= 1 {
            self.set_err("Cannot delete the only column");
            return;
        }
        let n = n.min(self.table.ncols() - 1);
        self.yank_cols(at, at + n - 1);
        for _ in 0..n {
            self.record(Change::RemoveCol { at });
            if at < self.widths.len() {
                self.widths.remove(at);
            }
        }
        self.col = self.col.min(self.last_col());
        self.set_msg(format!("{n} column{} deleted", if n == 1 { "" } else { "s" }));
    }

    /// Drop back to normal mode for a shortcut that works in every mode,
    /// committing an in-progress cell edit. Returns whether we were inserting.
    pub fn leave_mode(&mut self) -> bool {
        let inserting = self.mode == Mode::Insert;
        if inserting {
            self.commit_edit();
            if self.dot.in_insert && !self.dot.replaying {
                self.dot.last = std::mem::take(&mut self.dot.rec);
            }
            self.dot.in_insert = false;
        } else {
            self.dot.rec.clear();
        }
        self.pending.clear();
        self.after_x = None;
        self.mode = Mode::Normal;
        self.commit();
        inserting
    }

    /// Insert an empty row at `at` and move there, still editing if we were.
    pub fn add_row(&mut self, at: usize) {
        let inserting = self.leave_mode();
        self.insert_rows(at, vec![vec![String::new()]]);
        self.row = at;
        self.commit();
        self.set_msg(format!("Added row {} · ⌘Z to undo", at + 1));
        if inserting {
            self.begin_insert(true);
        }
    }

    /// Insert an empty column at `at` and move there, still editing if we were.
    pub fn add_col(&mut self, at: usize) {
        let inserting = self.leave_mode();
        self.insert_col(at);
        self.col = at;
        self.commit();
        self.set_msg(format!("Added column {} · ⌘Z to undo, ⌘− to delete", col_name(at)));
        if inserting {
            self.begin_insert(true);
        }
    }

    /// The rows or columns a structural shortcut acts on: the selection in
    /// visual mode, else the cursor's. (r0, r1, c0, c1), inclusive.
    fn target(&self) -> (usize, usize, usize, usize) {
        match self.mode {
            Mode::Visual { .. } => self.selection(),
            _ => (self.row, self.row, self.col, self.col),
        }
    }

    /// "column B “name”", or "columns B–D".
    fn cols_label(&self, c0: usize, c1: usize) -> String {
        if c0 != c1 {
            return format!("columns {}–{}", col_name(c0), col_name(c1));
        }
        let name = if self.header_active() { self.table.cell(0, c0) } else { "" };
        if name.is_empty() {
            format!("column {}", col_name(c0))
        } else {
            format!("column {} “{name}”", col_name(c0))
        }
    }

    /// Delete the selected rows (or the cursor row).
    pub fn remove_rows(&mut self) {
        let (r0, r1, _, _) = self.target();
        self.leave_mode();
        self.delete_rows(r0, r1);
        self.commit();
        let what = if r0 == r1 {
            format!("row {}", r0 + 1)
        } else {
            format!("rows {}–{}", r0 + 1, r1 + 1)
        };
        self.set_msg(format!("Deleted {what} · ⌘Z to undo"));
    }

    /// Delete the selected columns (or the cursor column).
    pub fn remove_cols(&mut self) {
        let (_, _, c0, c1) = self.target();
        self.leave_mode();
        if self.table.ncols() <= 1 {
            return self.set_err("Cannot delete the only column");
        }
        let c1 = c1.min(c0 + self.table.ncols() - 2);
        let what = self.cols_label(c0, c1);
        self.delete_cols(c0, c1 - c0 + 1);
        self.commit();
        self.col = c0.min(self.last_col());
        // A column click parks the cursor on row 0; keep the view where it was.
        if !self.header_active() {
            self.row = self.row.max(self.top);
        }
        self.set_msg(format!("Deleted {what} · ⌘Z to undo"));
    }

    // ---- registers / clipboard -------------------------------------------

    fn set_register(&mut self, reg: Register, cx: &mut App) {
        let (Register::Rows(cells) | Register::Block(cells)) = &reg else {
            return;
        };
        let mut text = cells
            .iter()
            .map(|r| r.join("\t"))
            .collect::<Vec<_>>()
            .join("\n");
        if matches!(reg, Register::Rows(_)) {
            text.push('\n');
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        self.clipboard_last = Some(text);
        self.register = reg;
    }

    pub fn yank_rows(&mut self, r0: usize, r1: usize) {
        let rows = self.rows_in(r0, r1).into_iter().map(|r| self.table.row_cells(r)).collect();
        self.pending_register = Some(Register::Rows(rows));
    }

    /// Yank the (shown) rows of a block.
    pub fn yank_block(&mut self, r0: usize, r1: usize, c0: usize, c1: usize) {
        self.yank_cells(self.rows_in(r0, r1), c0, c1);
    }

    /// Yank whole columns, hidden rows included.
    pub fn yank_cols(&mut self, c0: usize, c1: usize) {
        self.yank_cells((0..self.table.nrows()).collect(), c0, c1);
    }

    fn yank_cells(&mut self, rows: Vec<usize>, c0: usize, c1: usize) {
        let rows = rows
            .into_iter()
            .map(|r| {
                let mut f = self.table.fields(r).skip(c0);
                (c0..=c1)
                    .map(|_| f.next().unwrap_or("").to_owned())
                    .collect()
            })
            .collect();
        self.pending_register = Some(Register::Block(rows));
    }

    /// Registers are staged during a command and flushed with access to the
    /// app (for the system clipboard) once the command is done.
    pub fn flush_register(&mut self, cx: &mut App) {
        if let Some(reg) = self.pending_register.take() {
            self.set_register(reg, cx);
        }
    }

    /// The register to paste: the system clipboard wins if something else
    /// put text there since our last yank.
    pub fn paste_source(&mut self, cx: &mut App) -> Option<(bool, Vec<Vec<String>>)> {
        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text())
            && Some(&text) != self.clipboard_last.as_ref()
            && !text.is_empty()
        {
            let text = text.strip_suffix('\n').unwrap_or(&text);
            let rows = text
                .split('\n')
                .map(|l| {
                    l.strip_suffix('\r')
                        .unwrap_or(l)
                        .split('\t')
                        .map(str::to_owned)
                        .collect()
                })
                .collect();
            return Some((false, rows));
        }
        match &self.register {
            Register::Empty => None,
            Register::Rows(r) => Some((true, r.clone())),
            Register::Block(b) => Some((false, b.clone())),
        }
    }

    pub fn paste(&mut self, before: bool, cx: &mut App) {
        let Some((linewise, rows)) = self.paste_source(cx) else {
            self.set_err("Nothing to paste");
            return;
        };
        if linewise {
            let at = if before { self.row } else { self.row + 1 };
            let n = rows.len();
            self.insert_rows(at, rows);
            self.row = at;
            self.set_msg(format!("{n} more rows"));
        } else {
            let (r0, c0) = (self.row, self.col);
            for (i, cells) in rows.into_iter().enumerate() {
                if r0 + i > self.last_row() {
                    self.insert_rows(r0 + i, vec![vec![String::new()]]);
                }
                for (j, v) in cells.into_iter().enumerate() {
                    self.set_cell(r0 + i, c0 + j, v);
                }
            }
        }
    }

    // ---- column widths ---------------------------------------------------

    pub fn ensure_widths(&mut self) {
        let n = self.table.ncols();
        if self.widths.len() < n {
            self.widths.resize(n, 8);
        }
    }

    fn fit_cols(&self, cols: std::ops::Range<usize>) -> Vec<u16> {
        let mut w = vec![MIN_WIDTH; cols.len()];
        let n = self.table.nrows();
        // The head of the file plus an even sample of the rest.
        let head = n.min(2000);
        let step = (n / 2000).max(1);
        let sample = (0..head).chain((head..n).step_by(step));
        for r in sample {
            for (i, cell) in self.table.fields(r).skip(cols.start).take(cols.len()).enumerate() {
                let d = display_width(cell).min(MAX_AUTO_WIDTH as usize) as u16;
                w[i] = w[i].max(d);
            }
        }
        w
    }

    pub fn fit_all(&mut self) {
        self.widths = self.fit_cols(0..self.table.ncols());
    }

    pub fn fit_col(&mut self, c: usize) {
        self.ensure_widths();
        self.widths[c] = self.fit_cols(c..c + 1)[0];
    }

    pub fn resize_col(&mut self, c: usize, delta: i32) {
        self.ensure_widths();
        self.widths[c] = (self.widths[c] as i32 + delta).clamp(1, 500) as u16;
    }

    /// Column width in chars. The column being edited grows to fit the
    /// text (plus room for the caret) and falls back as text is deleted.
    pub fn col_chars(&self, c: usize) -> usize {
        let w = self.widths.get(c).copied().unwrap_or(8) as usize;
        if self.mode == Mode::Insert && c == self.col {
            w.max(display_width(&self.edit.text) + 1)
        } else {
            w
        }
    }

    fn col_px(&self, c: usize, m: &Metrics) -> f32 {
        self.col_chars(c) as f32 * m.char_w + 2.0 * PAD
    }

    // ---- viewport ----------------------------------------------------------

    fn ensure_metrics(&mut self, window: &mut Window) -> Metrics {
        if let Some(m) = self.metrics
            && m.font_size == self.font_size
        {
            return m;
        }
        let ts = window.text_system();
        let id = ts.resolve_font(&mono_font());
        let fs = px(self.font_size);
        let char_w = ts
            .advance(id, fs, 'm')
            .map(|s| s.width / px(1.0))
            .unwrap_or(self.font_size * 0.6);
        let m = Metrics {
            font_size: self.font_size,
            char_w,
            row_h: (self.font_size * 1.75).round(),
        };
        self.metrics = Some(m);
        m
    }

    fn layout_view(&mut self, width: f32, height: f32, m: &Metrics) {
        let digits = self.table.nrows().to_string().len().max(3);
        self.view.width = width;
        self.view.height = height;
        self.view.gutter_w = (digits + 2) as f32 * m.char_w;
        self.view.data_y = m.row_h * (1 + self.header_active() as usize) as f32;
        let data_h = height - 2.0 * m.row_h - self.view.data_y;
        self.view.rows = ((data_h / m.row_h).floor() as usize).max(1);
    }

    pub fn scroll_to_cursor(&mut self) {
        self.clamp_cursor();
        let first = self.first_data_row();
        let vis = self.page_rows();
        // A hidden cursor row snaps to the next shown one.
        let cur = self.disp_of(self.row);
        self.row = self.disp_row(cur);
        let mut top = self.disp_of(self.top);
        if cur >= first {
            if cur < top {
                top = cur;
            } else if cur >= top + vis {
                top = cur + 1 - vis;
            }
        }
        self.top = self.disp_row(top.clamp(first, (self.disp_len() - 1).max(first)));
        if let Some(m) = self.metrics {
            let avail = self.view.width - self.view.gutter_w;
            if self.col < self.left {
                self.left = self.col;
            }
            loop {
                let w: f32 = (self.left..=self.col).map(|c| self.col_px(c, &m)).sum();
                if w <= avail || self.left >= self.col {
                    break;
                }
                self.left += 1;
            }
        }
    }

    pub fn scroll_by(&mut self, rows: isize) {
        let first = self.first_data_row();
        let top = (self.disp_of(self.top) as isize + rows).max(first as isize) as usize;
        self.top = self.disp_row(top.min((self.disp_len() - 1).max(first)));
    }

    /// Keep the cursor inside the viewport after the viewport moved.
    pub fn cursor_into_view(&mut self) {
        let first = self.first_data_row();
        let mut cur = self.disp_of(self.row);
        if cur >= first {
            let top = self.disp_of(self.top);
            let bottom = (top + self.page_rows() - 1).min(self.disp_len() - 1);
            cur = cur.clamp(top, bottom.max(top));
        }
        self.row = self.disp_row(cur);
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = self.metrics else { return };
        let d = ev.delta.pixel_delta(px(m.row_h));
        self.scroll_acc.0 -= d.x / px(1.0);
        self.scroll_acc.1 -= d.y / px(1.0);
        let rows = (self.scroll_acc.1 / m.row_h).trunc();
        self.scroll_acc.1 -= rows * m.row_h;
        let col_step = m.char_w * 8.0;
        let cols = (self.scroll_acc.0 / col_step).trunc();
        self.scroll_acc.0 -= cols * col_step;
        if rows == 0.0 && cols == 0.0 {
            return;
        }
        self.scroll_by(rows as isize);
        self.left = (self.left as isize + cols as isize).clamp(0, self.last_col() as isize) as usize;
        self.cursor_into_view();
        if self.col < self.left {
            self.col = self.left;
        }
        self.follow = false;
        cx.notify();
    }

    fn hit_test(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let m = self.metrics?;
        let col = self
            .view
            .cols
            .iter()
            .find(|&&(_, x0, x1)| x >= x0 && x < x1)?
            .0;
        if self.header_active() && y >= m.row_h && y < 2.0 * m.row_h {
            return Some((0, col));
        }
        if y < self.view.data_y || (y - self.view.data_y) / m.row_h >= self.view.rows as f32 {
            return None;
        }
        let d = self.disp_of(self.top) + ((y - self.view.data_y) / m.row_h) as usize;
        (d < self.disp_len()).then(|| (self.disp_row(d), col))
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.after_x = None;
        let pos = ev.position;
        let (x, y) = (pos.x / px(1.0), pos.y / px(1.0));
        if let Some(m) = self.metrics
            && !matches!(self.mode, Mode::Command | Mode::Search { .. } | Mode::Filter)
        {
            if self.view.add_col.is_some_and(|(x0, x1)| x >= x0 && x < x1 && y < m.row_h) {
                self.add_col(self.table.ncols());
                self.follow = true;
                return cx.notify();
            }
            if self.view.add_row.is_some_and(|y0| x < self.view.gutter_w && y >= y0 && y < y0 + m.row_h) {
                self.add_row(self.table.nrows());
                self.follow = true;
                return cx.notify();
            }
        }
        if let Some(m) = self.metrics
            && matches!(self.mode, Mode::Normal | Mode::Visual { .. })
        {
            let extend = ev.modifiers.shift && matches!(self.mode, Mode::Visual { .. });
            let col_at = |x: f32| self.view.cols.iter().find(|&&(_, x0, x1)| x >= x0 && x < x1).map(|c| c.0);
            // Column letter: select the whole column (shift extends).
            if y < m.row_h
                && let Some(col) = col_at(x)
            {
                let anchor_col = if extend { self.anchor.1 } else { col };
                // Keep the cursor on the pinned header row so the view doesn't jump.
                self.anchor = (self.last_row(), anchor_col);
                (self.row, self.col) = (0, col);
                self.mode = Mode::Visual { line: false };
                self.set_msg(format!("{} selected · ⌘⇧⌫ deletes, ⌥←/⌥→ adds", self.cols_label(anchor_col.min(col), anchor_col.max(col))));
                self.follow = self.header_active();
                return cx.notify();
            }
            // Row number: select the whole row (shift extends).
            if x < self.view.gutter_w
                && let Some((row, _)) = self.hit_test(self.view.cols.first().map_or(x, |c| c.1), y)
            {
                if !extend {
                    self.anchor = (row, self.col);
                }
                self.row = row;
                self.mode = Mode::Visual { line: true };
                self.set_msg("Row selected · ⌘⌫ deletes, ⌥↑/⌥↓ adds");
                self.follow = true;
                return cx.notify();
            }
        }
        let Some((row, col)) = self.hit_test(x, y) else {
            return;
        };
        match self.mode {
            Mode::Insert => {
                self.commit_edit();
                self.row = row;
                self.col = col;
                self.begin_insert(true);
            }
            Mode::Command | Mode::Search { .. } | Mode::Filter => {}
            Mode::Visual { .. } => (self.row, self.col) = (row, col),
            Mode::Normal => {
                if ev.modifiers.shift {
                    self.anchor = (self.row, self.col);
                    self.mode = Mode::Visual { line: false };
                }
                (self.row, self.col) = (row, col);
                if ev.click_count >= 2 {
                    self.begin_insert(true);
                }
            }
        }
        self.follow = true;
        cx.notify();
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(token) = key_token(&ev.keystroke) else {
            return;
        };
        cx.stop_propagation();
        self.handle_key(&token, cx);
        self.flush_register(cx);
        self.follow = true;
        cx.notify();
    }

    // ---- insert-mode helpers ---------------------------------------------

    pub fn begin_insert(&mut self, at_end: bool) {
        self.clamp_cursor();
        self.edit = LineEdit::new(self.cell().to_owned(), at_end);
        self.mode = Mode::Insert;
    }

    pub fn commit_edit(&mut self) {
        let text = std::mem::take(&mut self.edit.text);
        // Keep the width the column grew to while typing (auto-fit's cap is
        // for loaded data, not text you just typed).
        let typed = display_width(&text).min(500) as u16;
        self.set_cell(self.row, self.col, text);
        self.ensure_widths();
        let w = &mut self.widths[self.col];
        *w = (*w).max(typed);
        self.edit.cursor = 0;
    }

    // ---- painting ----------------------------------------------------------

    fn build_frame(&mut self, m: &Metrics) -> Frame {
        let mut f = Frame::new(*m);
        let (w, h) = (self.view.width, self.view.height);
        let rh = m.row_h;
        let gw = self.view.gutter_w;
        let header = self.header_active();
        // The current mode's color tints the cursor, active column, current
        // line number and status bar.
        let accent = match self.mode {
            Mode::Normal => NORMAL_C,
            Mode::Insert => INSERT_C,
            Mode::Visual { .. } => VISUAL_C,
            Mode::Command | Mode::Search { .. } => COMMAND_C,
            Mode::Filter => FILTER_C,
        };
        let tint = |a: f32| c(accent).opacity(a);

        f.fill(0.0, 0.0, w, h, BG);

        // Visible columns.
        let mut cols = Vec::new();
        let mut x = gw;
        for col in self.left..self.table.ncols() {
            let cw = self.col_px(col, m);
            cols.push((col, x, x + cw));
            x += cw;
            if x >= w {
                break;
            }
        }
        let grid_right = x.min(w);

        // Visible rows: (row, y).
        let mut rows = Vec::with_capacity(self.view.rows + 1);
        if header {
            rows.push((0, rh));
        }
        let top = self.disp_of(self.top);
        for i in 0..self.view.rows {
            if top + i >= self.disp_len() {
                break;
            }
            rows.push((self.disp_row(top + i), self.view.data_y + i as f32 * rh));
        }
        let grid_bottom = rows.last().map_or(rh, |&(_, y)| y + rh);

        let sel = matches!(self.mode, Mode::Visual { .. }).then(|| self.selection());
        let in_sel = |r: usize, c: usize| {
            sel.is_some_and(|(r0, r1, c0, c1)| r >= r0 && r <= r1 && c >= c0 && c <= c1)
        };
        let search = self.search.as_ref().filter(|_| self.highlight).map(|s| &s.re);
        let filter = (!self.filter.trim().is_empty()).then_some(self.filter.as_str());

        // Column bar, with the active column as a pill.
        f.vgradient(0.0, 0.0, w, rh, BAR_TOP, BG_DARK);
        for &(col, x0, x1) in &cols {
            let active = col == self.col;
            if active {
                f.round(x0 + 3.0, 3.0, x1 - x0 - 6.0, rh - 6.0, (rh - 6.0) / 2.0, tint(0.18));
            }
            let name = col_name(col);
            let tw = display_width(&name) as f32 * m.char_w;
            let color = if active { accent } else { COMMENT };
            f.text(name, x0 + ((x1 - x0) - tw) / 2.0, 0.0, color, active)
                .clip(x0, 0.0, x1 - x0, rh);
        }

        // "+" buttons after the last column and below the last row.
        self.view.add_col = None;
        let bw = 3.0 * m.char_w;
        if cols.last().is_some_and(|&(c, _, _)| c == self.last_col()) && grid_right + bw <= w {
            f.ring(rect(grid_right + 4.0, 4.0, bw - 8.0, rh - 8.0), 4.0, 1.0, c(FG_GUTTER));
            f.text("+", grid_right + (bw - m.char_w) / 2.0, 0.0, COMMENT, true);
            self.view.add_col = Some((grid_right, grid_right + bw));
        }
        self.view.add_row = None;
        if rows.last().is_some_and(|&(r, _)| r == self.last_row()) && grid_bottom + rh <= h - 2.0 * rh {
            let bx = (gw - bw) / 2.0;
            f.ring(rect(bx + 4.0, grid_bottom + 4.0, bw - 8.0, rh - 8.0), 4.0, 1.0, c(FG_GUTTER));
            f.text("+", (gw - m.char_w) / 2.0, grid_bottom, COMMENT, true);
            self.view.add_row = Some(grid_bottom);
        }

        // Rows: header gradient, zebra stripes, current-row highlight.
        let digits = ((gw / m.char_w) as usize).saturating_sub(2);
        let cur_disp = self.disp_of(self.row);
        for &(r, y) in &rows {
            let is_header = header && r == 0;
            let d = self.disp_of(r);
            if is_header {
                f.vgradient(gw, y, grid_right - gw, rh, HEADER_TOP, HEADER_BOTTOM);
            } else if r == self.row {
                f.fill(gw, y, grid_right - gw, rh, BG_HIGHLIGHT);
            } else if d % 2 == 1 {
                f.fill(gw, y, grid_right - gw, rh, BG_STRIPE);
            }
            if r == self.row {
                f.round(1.0, y + 4.0, 3.0, rh - 8.0, 1.5, accent);
            }
            let num = if self.relative && r != self.row {
                d.abs_diff(cur_disp)
            } else {
                r + 1
            };
            f.text(
                format!("{num:>digits$}"),
                m.char_w,
                y,
                if r == self.row { accent } else { FG_GUTTER },
                r == self.row,
            );

            let mut fields = self.table.fields(r).skip(self.left);
            for &(col, x0, x1) in &cols {
                let mut cell = fields.next().unwrap_or("");
                if !is_header && filter.is_some_and(|q| !fuzzy_match(q, cell)) {
                    cell = "";
                }
                let cw = x1 - x0;
                let cursor = r == self.row && col == self.col;
                if in_sel(r, col) {
                    f.fill(x0, y, cw, rh, tint(0.2));
                } else if cursor {
                    f.fill(x0, y, cw, rh, tint(0.14));
                } else if search.is_some_and(|re| !cell.is_empty() && re.is_match(cell)) {
                    f.round(x0 + 2.0, y + 2.0, cw - 4.0, rh - 4.0, 3.0, c(TEAL).opacity(0.22));
                }
                if cell.is_empty() || (cursor && self.mode == Mode::Insert) {
                    continue;
                }
                let max_chars = ((cw - 2.0 * PAD) / m.char_w).floor().max(0.0) as usize;
                let text = truncate(cell, max_chars);
                let numeric = !is_header && looks_numeric(cell);
                let color = if is_header {
                    HEADER_FG
                } else if numeric {
                    NUM_FG
                } else {
                    FG
                };
                let t = f.text(text, x0 + PAD, y, color, is_header).clip(x0, y, cw, rh);
                if numeric {
                    t.right = Some(x1 - PAD);
                }
            }
        }

        // Grid: faint column rules (rows are told apart by the stripes), and
        // an accent line under the header that fades out to the right.
        for &(_, _, x1) in &cols {
            f.fill(x1 - 1.0, rh, 1.0, grid_bottom - rh, GRID);
        }
        f.fill(gw - 1.0, rh, 1.0, grid_bottom - rh, GRID);
        if header {
            f.hgradient(gw, 2.0 * rh - 1.0, grid_right - gw, 1.0, tint(0.7), tint(0.05));
        }

        // Cursor: rounded ring with a soft glow, or the insert overlay.
        let cursor_y = rows.iter().find(|&&(r, _)| r == self.row).map(|&(_, y)| y);
        let cursor_x = cols.iter().find(|&&(c, _, _)| c == self.col);
        if let (Some(y), Some(&(_, x0, x1))) = (cursor_y, cursor_x) {
            // The column itself grows while inserting; only clip to the window.
            let cw = x1.min(w) - x0;
            f.ring(rect(x0 - 4.0, y - 4.0, cw + 8.0, rh + 8.0), 8.0, 2.0, tint(0.07));
            f.ring(rect(x0 - 2.0, y - 2.0, cw + 4.0, rh + 4.0), 6.0, 2.0, tint(0.2));
            if self.mode == Mode::Insert {
                f.round(x0, y, cw, rh, 4.0, c(BG_DARK));
                let text = &self.edit.text;
                let avail = (((cw - 2.0 * PAD) / m.char_w).floor() as usize).saturating_sub(1);
                let caret_ci = text[..self.edit.cursor].chars().count();
                let start_ci = caret_ci.saturating_sub(avail);
                let start = text.char_indices().nth(start_ci).map_or(text.len(), |(i, _)| i);
                let shown: String = text[start..].chars().take(avail.max(1)).collect();
                let caret = self.edit.cursor - start;
                f.text(shown, x0 + PAD, y, FG, false)
                    .clip(x0, y, cw, rh)
                    .caret = Some((caret, accent));
            }
            f.ring(rect(x0, y, cw, rh), 4.0, 1.5, c(accent));
        }

        self.view.cols = cols;
        self.build_status(&mut f, m, accent);
        f
    }

    fn build_status(&self, f: &mut Frame, m: &Metrics, accent: u32) {
        let (w, h, rh) = (self.view.width, self.view.height, m.row_h);
        let sy = h - 2.0 * rh;
        let cw = m.char_w;
        f.fill(0.0, sy, w, 2.0 * rh, BG_DARK);
        f.fill(0.0, sy, w, 1.0, GRID);

        // Left: mode pill, file name, a dot when modified.
        let label = match self.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual { line: false } => "VISUAL",
            Mode::Visual { line: true } => "V-LINE",
            Mode::Command | Mode::Search { .. } => "COMMAND",
            Mode::Filter => "FILTER",
        };
        let pill_h = rh - 8.0;
        let lw = (label.len() + 2) as f32 * cw;
        f.round(cw / 2.0, sy + 4.0, lw, pill_h, pill_h / 2.0, accent);
        f.text(label, cw * 1.5, sy, BLACK, true);
        let name = self
            .path
            .as_ref()
            .map_or("[No Name]".into(), |p| p.display().to_string());
        let nx = cw / 2.0 + lw + cw;
        f.text(name.clone(), nx, sy, FG_DARK, false);
        if self.is_dirty() {
            f.text("●", nx + (display_width(&name) + 1) as f32 * cw, sy, accent, false);
        }

        // Right: position, then the cell reference as a tinted pill.
        let cell = format!("{}{}", col_name(self.col), self.row + 1);
        let zw = (cell.len() + 2) as f32 * cw;
        let zx = w - zw - cw / 2.0;
        f.round(zx, sy + 4.0, zw, pill_h, pill_h / 2.0, c(accent).opacity(0.18));
        f.text(cell, zx + cw, sy, accent, true);
        let pos = format!(
            "row {}/{} · col {}/{}",
            self.row + 1,
            self.table.nrows(),
            self.col + 1,
            self.table.ncols()
        );
        let pos_x = zx - cw;
        f.text(pos.clone(), 0.0, sy, COMMENT, false).right = Some(pos_x);
        let yw = display_width(&pos) as f32 * cw;

        let mut extra = String::new();
        if !self.pending.is_empty() {
            extra.push_str(&self.pending.concat());
            extra.push_str("   ");
        }
        if let Mode::Visual { .. } = self.mode {
            let (r0, r1, c0, c1) = self.selection();
            extra.push_str(&format!("{}×{} ", r1 - r0 + 1, c1 - c0 + 1));
        }
        if let Some(shown) = &self.shown {
            let n = shown.len() - self.header_active() as usize;
            let total = self.table.nrows() - self.header_active() as usize;
            if self.mode != Mode::Filter {
                extra.push_str(&format!("filter: {}  ", self.filter));
            }
            extra.push_str(&format!("{n} of {total} rows "));
        }
        f.text(extra, 0.0, sy, FG_DARK, false).right = Some(pos_x - yw - 2.0 * cw);

        // Command line / message / cell preview.
        let cy = h - rh;
        let prefix = match self.mode {
            Mode::Command => Some(":"),
            Mode::Search { forward: true } => Some("/"),
            Mode::Search { forward: false } => Some("?"),
            Mode::Filter => Some("filter: "),
            _ => None,
        };
        if let Some(p) = prefix {
            let t = f.text(format!("{p}{}", self.cmdline.text), m.char_w, cy, FG, false);
            t.caret = Some((p.len() + self.cmdline.cursor, accent));
        } else if let Some(msg) = &self.message {
            f.text(
                msg.text.clone(),
                m.char_w,
                cy,
                if msg.error { RED } else { FG },
                false,
            );
        } else {
            let max = ((w / m.char_w) as usize).saturating_sub(12);
            let label = format!("{}{} ", col_name(self.col), self.row + 1);
            let lw = (label.len() + 1) as f32 * m.char_w;
            f.text(label, m.char_w, cy, COMMENT, true);
            f.text(truncate(self.cell(), max), lw, cy, FG, false);
        }
    }
}

pub fn mono_font() -> Font {
    #[cfg(target_os = "macos")]
    let family = "Menlo";
    #[cfg(not(target_os = "macos"))]
    let family = "DejaVu Sans Mono";
    font(family)
}

/// Turn a gpui keystroke into a vim-style token: "j", "G", "<c-d>", "<esc>".
pub fn key_token(ks: &Keystroke) -> Option<String> {
    let m = &ks.modifiers;
    let named = match ks.key.as_str() {
        "escape" => Some("esc"),
        "enter" => Some("cr"),
        "backspace" => Some("bs"),
        "tab" if m.shift => Some("s-tab"),
        "tab" => Some("tab"),
        "delete" => Some("del"),
        "left" => Some("left"),
        "right" => Some("right"),
        "up" => Some("up"),
        "down" => Some("down"),
        "home" => Some("home"),
        "end" => Some("end"),
        "pageup" => Some("pageup"),
        "pagedown" => Some("pagedown"),
        _ => None,
    };
    if m.platform {
        let alt = if m.alt { "a-" } else { "" };
        let shift = if m.shift { "s-" } else { "" };
        return Some(format!("<d-{alt}{shift}{}>", ks.key));
    }
    if m.control {
        return Some(format!("<c-{}>", ks.key));
    }
    if let Some(n) = named {
        let alt = if m.alt { "a-" } else { "" };
        return Some(format!("<{alt}{n}>"));
    }
    match &ks.key_char {
        Some(ch) if !ch.is_empty() && !ch.chars().any(char::is_control) => Some(ch.clone()),
        _ if ks.key == "space" => Some(" ".into()),
        _ => None,
    }
}

/// True for tokens that stand for text rather than a named key.
pub fn is_text(token: &str) -> bool {
    !(token.len() > 2 && token.starts_with('<') && token.ends_with('>'))
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let m = self.ensure_metrics(window);
        let vp = window.viewport_size();
        self.layout_view(vp.width / px(1.0), vp.height / px(1.0), &m);
        if self.follow {
            self.scroll_to_cursor();
        }
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("[No Name]".into(), |n| n.to_string_lossy().into_owned());
        let title = format!("{name}{} — tsv", if self.is_dirty() { " ●" } else { "" });
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
        let frame = self.build_frame(&m);
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .child(
                canvas(|_, _, _| {}, move |bounds, _, window, cx| frame.paint(bounds, window, cx))
                    .size_full(),
            )
    }
}

// ---- frame: a display list built in render, painted in the canvas ---------

fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
}

/// Anything a quad can be filled with: a palette color, or a translucent one.
trait Paint {
    fn background(self) -> Background;
}

impl Paint for u32 {
    fn background(self) -> Background {
        c(self).into()
    }
}

impl Paint for Hsla {
    fn background(self) -> Background {
        self.into()
    }
}

struct TextItem {
    text: SharedString,
    x: f32,
    y: f32,
    color: Hsla,
    bold: bool,
    right: Option<f32>,
    clip: Option<Bounds<Pixels>>,
    caret: Option<(usize, u32)>,
}

impl TextItem {
    fn clip(&mut self, x: f32, y: f32, w: f32, h: f32) -> &mut Self {
        self.clip = Some(Bounds::new(point(px(x), px(y)), size(px(w), px(h))));
        self
    }
}

struct Frame {
    m: Metrics,
    quads: Vec<PaintQuad>,
    texts: Vec<TextItem>,
}

impl Frame {
    fn new(m: Metrics) -> Self {
        Self {
            m,
            quads: Vec::with_capacity(512),
            texts: Vec::with_capacity(1024),
        }
    }

    fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, color: impl Paint) {
        self.quads.push(fill(rect(x, y, w, h), color.background()));
    }

    fn round(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: impl Paint) {
        self.quads
            .push(fill(rect(x, y, w, h), color.background()).corner_radii(px(r)));
    }

    /// A rounded outline `t` px thick.
    fn ring(&mut self, b: Bounds<Pixels>, r: f32, t: f32, color: Hsla) {
        self.quads.push(quad(
            b,
            px(r),
            transparent_black(),
            px(t),
            color,
            BorderStyle::default(),
        ));
    }

    fn vgradient(&mut self, x: f32, y: f32, w: f32, h: f32, top: u32, bottom: u32) {
        let g = linear_gradient(180.0, linear_color_stop(c(top), 0.0), linear_color_stop(c(bottom), 1.0));
        self.quads.push(fill(rect(x, y, w, h), g));
    }

    fn hgradient(&mut self, x: f32, y: f32, w: f32, h: f32, left: Hsla, right: Hsla) {
        let g = linear_gradient(90.0, linear_color_stop(left, 0.0), linear_color_stop(right, 1.0));
        self.quads.push(fill(rect(x, y, w, h), g));
    }

    fn text(
        &mut self,
        text: impl Into<SharedString>,
        x: f32,
        y: f32,
        color: u32,
        bold: bool,
    ) -> &mut TextItem {
        self.texts.push(TextItem {
            text: text.into(),
            x,
            y,
            color: c(color),
            bold,
            right: None,
            clip: None,
            caret: None,
        });
        self.texts.last_mut().unwrap()
    }

    fn paint(self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let origin = bounds.origin;
        for mut q in self.quads {
            q.bounds.origin += origin;
            window.paint_quad(q);
        }
        let regular = mono_font();
        let mut bold = mono_font();
        bold.weight = FontWeight::BOLD;
        let fs = px(self.m.font_size);
        let rh = px(self.m.row_h);
        for t in self.texts {
            if t.text.is_empty() && t.caret.is_none() {
                continue;
            }
            let run = TextRun {
                len: t.text.len(),
                font: if t.bold { bold.clone() } else { regular.clone() },
                color: t.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window.text_system().shape_line(t.text, fs, &[run], None);
            let x = match t.right {
                Some(r) => r - line.width / px(1.0),
                None => t.x,
            };
            let pos = point(px(x), px(t.y)) + origin;
            let caret = t.caret.map(|(ix, color)| {
                let cx = line.x_for_index(ix);
                fill(
                    Bounds::new(
                        point(pos.x + cx, pos.y + px(3.0)),
                        size(px(2.0), rh - px(6.0)),
                    ),
                    c(color),
                )
            });
            let paint = |window: &mut Window, cx: &mut App| {
                let _ = line.paint(pos, rh, window, cx);
                if let Some(q) = caret {
                    window.paint_quad(q);
                }
            };
            match t.clip {
                Some(mut clip) => {
                    clip.origin += origin;
                    window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |w| {
                        paint(w, cx)
                    });
                }
                None => paint(window, cx),
            }
        }
    }
}
