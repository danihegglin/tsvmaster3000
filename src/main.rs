//! tsv — a fast TUI viewer/editor for tab-separated files.
//!
//! Usage: tsv <file.tsv>

use std::{env, fs, io, path::PathBuf};

use ratatui::{
    crossterm::{
        event::{
            self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
            KeyModifiers,
        },
        execute,
    },
    prelude::*,
    widgets::Paragraph,
    DefaultTerminal,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Hard cap so one pathological cell cannot eat the whole viewport.
const MAX_COL: usize = 48;
const MIN_COL: usize = 3;
const PALETTE: [Color; 6] = [
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Magenta,
    Color::Blue,
    Color::LightRed,
];

#[derive(PartialEq, Eq, Clone, Copy)]
enum Mode {
    Normal,
    Insert,
    Command,
    Filter,
}

/// Live row filter. A row stays if every term occurs in one of its cells;
/// cells that contain one of the terms are highlighted.
struct Filter {
    query: String,
    terms: Vec<String>,
    /// Smartcase: an uppercase letter in the query makes it case-sensitive.
    case: bool,
    /// Matching data rows (never the header), ascending.
    rows: Vec<usize>,
}

impl Filter {
    fn new(query: String) -> Self {
        let case = query.chars().any(char::is_uppercase);
        let terms = query
            .split_whitespace()
            .map(|t| if case { t.to_owned() } else { t.to_lowercase() })
            .collect();
        Self { query, terms, case, rows: Vec::new() }
    }

    fn fold<'a>(&self, s: &'a str) -> std::borrow::Cow<'a, str> {
        if self.case {
            s.into()
        } else {
            s.to_lowercase().into()
        }
    }

    fn cell_hit(&self, s: &str) -> bool {
        let s = self.fold(s);
        self.terms.iter().any(|t| s.contains(t.as_str()))
    }

    fn row_hit(&self, row: &[String]) -> bool {
        let cells: Vec<_> = row.iter().map(|c| self.fold(c)).collect();
        self.terms.iter().all(|t| cells.iter().any(|c| c.contains(t.as_str())))
    }
}

/// A reversible edit. `App::exec` applies one and returns its inverse, so the
/// undo and redo stacks just hold whatever undoes the last step.
enum Change {
    SetCell { r: usize, c: usize, value: String },
    InsertRow { r: usize, row: Vec<String> },
    DeleteRow { r: usize },
    InsertCol { c: usize, col: Vec<String> },
    DeleteCol { c: usize },
    /// Several changes as one undo step; the cursor ends up on `at`.
    Batch { at: (usize, usize), changes: Vec<Change> },
}

struct Entry {
    /// Stays with the edit across undo/redo, so we can tell whether the grid
    /// is back at the saved state.
    id: u64,
    change: Change,
}

/// First key of a two-key normal-mode command.
#[derive(Clone, Copy)]
enum Pending {
    T,
    D,
    /// `x` already cleared the cell; `cleared` says whether that made an
    /// undo entry that `xx` should take back.
    X { cleared: bool },
}

struct App {
    path: PathBuf,
    /// Row 0 is the header, rows 1.. are data.
    rows: Vec<Vec<String>>,
    widths: Vec<usize>,
    cur: (usize, usize),
    off: (usize, usize),
    mode: Mode,
    edit: String,
    caret: usize, // byte offset into `edit`
    cmd: String,
    pending: Option<Pending>,
    filter: Option<Filter>,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    next_id: u64,
    /// Id of the newest undo entry when the file was last written.
    saved: Option<u64>,
    status: String,
    quit: bool,
}

fn main() -> io::Result<()> {
    let Some(arg) = env::args_os().nth(1) else {
        eprintln!("usage: tsv <file.tsv>");
        std::process::exit(2);
    };
    let mut app = App::load(PathBuf::from(arg))?;
    let mut terminal = ratatui::init();
    // Terminal pastes then arrive as one `Event::Paste` instead of keystrokes.
    // Not every terminal supports it; `p` reads the clipboard directly.
    let _ = execute!(io::stdout(), EnableBracketedPaste);
    let res = app.run(&mut terminal);
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    res
}

impl App {
    fn load(path: PathBuf) -> io::Result<Self> {
        let raw = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let mut rows: Vec<Vec<String>> = raw
            .lines()
            .map(|l| l.split('\t').map(str::to_owned).collect())
            .collect();
        if rows.is_empty() {
            rows.push(vec![String::new()]);
        }
        let ncols = rows.iter().map(Vec::len).max().unwrap_or(1);
        for r in &mut rows {
            r.resize(ncols, String::new());
        }
        let mut app = Self {
            path,
            rows,
            widths: vec![0; ncols],
            cur: (0, 0),
            off: (0, 0),
            mode: Mode::Normal,
            edit: String::new(),
            caret: 0,
            cmd: String::new(),
            pending: None,
            filter: None,
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 0,
            saved: None,
            status: String::new(),
            quit: false,
        };
        app.measure_all();
        app.status = format!("{} rows × {} cols — :h for help", app.rows.len() - 1, ncols);
        Ok(app)
    }

    fn ncols(&self) -> usize {
        self.widths.len()
    }

    /// Recompute the natural width of a single column.
    fn measure(&mut self, col: usize) {
        let w = self
            .rows
            .iter()
            .filter_map(|r| r.get(col))
            .map(|c| c.width())
            .max()
            .unwrap_or(0);
        self.widths[col] = w.clamp(MIN_COL, MAX_COL);
    }

    fn measure_all(&mut self) {
        for c in 0..self.ncols() {
            self.measure(c);
        }
    }

    fn dirty(&self) -> bool {
        self.undo.last().map(|e| e.id) != self.saved
    }

    fn save(&mut self) {
        let mut out = String::with_capacity(self.rows.len() * 32);
        for r in &self.rows {
            out.push_str(&r.join("\t"));
            out.push('\n');
        }
        match fs::write(&self.path, out) {
            Ok(()) => {
                self.saved = self.undo.last().map(|e| e.id);
                self.status = format!("wrote {}", self.path.display());
            }
            Err(e) => self.status = format!("error: {e}"),
        }
    }

    fn run(&mut self, term: &mut DefaultTerminal) -> io::Result<()> {
        while !self.quit {
            term.draw(|f| self.draw(f))?;
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => self.on_key(k),
                Event::Paste(text) => self.on_paste(&text),
                _ => {}
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- input

    fn on_key(&mut self, k: KeyEvent) {
        match self.mode {
            Mode::Normal => self.normal(k),
            Mode::Insert => self.insert(k),
            Mode::Command => self.command(k),
            Mode::Filter => self.filter_key(k),
        }
    }

    fn on_paste(&mut self, text: &str) {
        match self.mode {
            Mode::Normal => self.paste(text),
            // Plain text goes in at the caret; anything with tabs or line
            // breaks commits the cell and is pasted as a block from there.
            Mode::Insert if text.contains(['\t', '\n', '\r']) => {
                self.commit(0);
                self.paste(text);
            }
            Mode::Insert => {
                self.edit.insert_str(self.caret, text);
                self.caret += text.len();
            }
            Mode::Command => self.cmd.push_str(text.lines().next().unwrap_or("")),
            Mode::Filter => {
                let mut query = self.filter.as_ref().map(|f| f.query.clone()).unwrap_or_default();
                query.push_str(&text.replace(['\t', '\n', '\r'], " "));
                self.set_filter(query);
            }
        }
    }

    fn filter_key(&mut self, k: KeyEvent) {
        let mut query = self.filter.as_ref().map(|f| f.query.clone()).unwrap_or_default();
        match k.code {
            KeyCode::Esc => {
                self.filter = None;
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                if query.is_empty() {
                    self.filter = None;
                } else {
                    self.status = "filter kept — / to change, Esc to clear".into();
                }
                return;
            }
            KeyCode::Backspace => {
                query.pop();
            }
            KeyCode::Char(c) => query.push(c),
            _ => return,
        }
        self.set_filter(query);
    }

    /// Replace the filter query and bring the cursor to a matching row.
    fn set_filter(&mut self, query: String) {
        self.filter = Some(Filter::new(query));
        self.refilter();
        let f = self.filter.as_ref().unwrap();
        let rows = &f.rows;
        if !f.terms.is_empty() && rows.binary_search(&self.cur.0).is_err() {
            self.cur.0 = rows.first().copied().unwrap_or(0);
        }
    }

    /// Recompute which rows match, after the query or the grid changed.
    fn refilter(&mut self) {
        let Some(f) = self.filter.as_mut() else { return };
        f.rows = if f.terms.is_empty() {
            (1..self.rows.len()).collect()
        } else {
            (1..self.rows.len()).filter(|&r| f.row_hit(&self.rows[r])).collect()
        };
    }

    /// Data rows on screen while filtering, ascending: the matches plus the
    /// cursor's row, so a row doesn't vanish while you're working on it.
    fn visible(&self) -> Option<Vec<usize>> {
        let f = self.filter.as_ref()?;
        let mut rows = f.rows.clone();
        if self.cur.0 > 0 {
            if let Err(i) = rows.binary_search(&self.cur.0) {
                rows.insert(i, self.cur.0);
            }
        }
        Some(rows)
    }

    fn normal(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match (self.pending.take(), k.code) {
            (Some(Pending::T), KeyCode::Char('t')) => return self.add_column(),
            (Some(Pending::D), KeyCode::Char('d')) => return self.delete_row(),
            (Some(Pending::X { cleared }), KeyCode::Char('x')) => {
                if cleared {
                    self.undo();
                }
                return self.delete_column();
            }
            // Anything else ends the pending command and runs as usual.
            _ => {}
        }
        match k.code {
            KeyCode::Char('s') if ctrl => self.save(),
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Char('r') if ctrl => self.redo(),
            KeyCode::Char('v') if ctrl => self.paste_clipboard(),
            KeyCode::Char('p') => self.paste_clipboard(),
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char('t') => self.pending = Some(Pending::T),
            KeyCode::Char('d') => self.pending = Some(Pending::D),
            KeyCode::Up | KeyCode::Char('k') => self.move_to(-1, 0),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Enter => self.move_to(1, 0),
            KeyCode::Left | KeyCode::Char('h') => self.move_to(0, -1),
            KeyCode::Right | KeyCode::Char('l') => self.move_to(0, 1),
            KeyCode::Tab => self.move_to(0, 1),
            KeyCode::BackTab => self.move_to(0, -1),
            KeyCode::PageDown => self.move_to(20, 0),
            KeyCode::PageUp => self.move_to(-20, 0),
            KeyCode::Home | KeyCode::Char('0') => self.cur.1 = 0,
            KeyCode::End | KeyCode::Char('$') => self.cur.1 = self.ncols() - 1,
            KeyCode::Char('g') => self.cur.0 = 0,
            KeyCode::Char('G') => {
                let last = self.visible().map_or(self.rows.len() - 1, |v| v.last().copied().unwrap_or(0));
                self.cur.0 = last;
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Filter;
                let query = self.filter.take().map(|f| f.query).unwrap_or_default();
                self.set_filter(query);
            }
            KeyCode::Esc if self.filter.is_some() => {
                self.filter = None;
                self.status = "filter cleared".into();
            }
            KeyCode::Char('i') => self.begin_edit(false),
            KeyCode::Char('a') => self.begin_edit(true),
            // Clear right away; a second `x` turns it into a column delete.
            KeyCode::Char('x') => {
                let (r, c) = self.cur;
                let cleared = !self.rows[r][c].is_empty();
                if cleared {
                    self.record(Change::SetCell { r, c, value: String::new() });
                }
                self.pending = Some(Pending::X { cleared });
            }
            KeyCode::Char('o') => {
                let r = (self.cur.0 + 1).max(1);
                let row = vec![String::new(); self.ncols()];
                self.record(Change::InsertRow { r, row });
            }
            KeyCode::Char('D') => self.delete_row(),
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.cmd = ":".into();
            }
            KeyCode::Char('q') => {
                if self.dirty() {
                    self.status = "unsaved changes — :w to write, :q! to discard".into();
                } else {
                    self.quit = true;
                }
            }
            _ => {}
        }
    }

    fn insert(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => self.commit(0),
            KeyCode::Enter => self.commit(1),
            // Commit and keep editing in the neighbouring cell; Tab past the
            // last column adds a new one.
            KeyCode::Tab | KeyCode::BackTab => {
                self.commit(0);
                if k.code == KeyCode::BackTab {
                    self.move_to(0, -1);
                } else if self.cur.1 + 1 == self.ncols() {
                    self.add_column();
                } else {
                    self.move_to(0, 1);
                }
                self.begin_edit(true);
            }
            // Commit and keep editing the cell above / below.
            KeyCode::Up | KeyCode::Down => {
                self.commit(0);
                self.move_to(if k.code == KeyCode::Up { -1 } else { 1 }, 0);
                self.begin_edit(true);
            }
            KeyCode::Char(c) => {
                self.edit.insert(self.caret, c);
                self.caret += c.len_utf8();
            }
            KeyCode::Backspace => {
                if let Some(c) = self.edit[..self.caret].chars().next_back() {
                    self.caret -= c.len_utf8();
                    self.edit.remove(self.caret);
                }
            }
            KeyCode::Delete => {
                if self.caret < self.edit.len() {
                    self.edit.remove(self.caret);
                }
            }
            KeyCode::Left => {
                if let Some(c) = self.edit[..self.caret].chars().next_back() {
                    self.caret -= c.len_utf8();
                }
            }
            KeyCode::Right => {
                if let Some(c) = self.edit[self.caret..].chars().next() {
                    self.caret += c.len_utf8();
                }
            }
            KeyCode::Home => self.caret = 0,
            KeyCode::End => self.caret = self.edit.len(),
            _ => {}
        }
    }

    fn command(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.cmd.clear();
            }
            KeyCode::Char(c) => self.cmd.push(c),
            KeyCode::Backspace => {
                self.cmd.pop();
                if self.cmd.is_empty() {
                    self.mode = Mode::Normal;
                }
            }
            KeyCode::Enter => {
                let cmd = std::mem::take(&mut self.cmd);
                self.mode = Mode::Normal;
                match cmd.as_str() {
                    ":w" => self.save(),
                    ":wq" | ":wq!" | ":x" | ":x!" => {
                        self.save();
                        self.quit = !self.dirty();
                    }
                    ":q" => {
                        if self.dirty() {
                            self.status = "unsaved changes — :q! to discard".into();
                        } else {
                            self.quit = true;
                        }
                    }
                    ":q!" => self.quit = true,
                    ":h" => {
                        self.status =
                            "arrows/hjkl/Tab move · i,a edit · Esc commit · x clear · o new row \
                             · dd delete row · tt/xx add/delete column · u/^R undo/redo · / filter \
                             · g,G,0,$ jump · :N go to row · :w :q :wq"
                                .into()
                    }
                    ":$" => self.cur.0 = self.rows.len() - 1,
                    // `:N` jumps to row N as numbered in the gutter (0 = header).
                    other => match other[1..].parse::<usize>() {
                        Ok(n) => self.cur.0 = n.min(self.rows.len() - 1),
                        Err(_) => self.status = format!("unknown command: {other}"),
                    },
                }
            }
            _ => {}
        }
    }

    // ----------------------------------------------------------------- edit

    /// Apply `ch`, put the cursor on it, and return the change that undoes it.
    fn exec(&mut self, ch: Change) -> Change {
        self.apply(ch, true)
    }

    /// `exec`, but with `remeasure` false the column widths may be left too
    /// wide; a batch measures once at the end instead of after every cell.
    fn apply(&mut self, ch: Change, remeasure: bool) -> Change {
        match ch {
            Change::SetCell { r, c, value } => {
                let old = std::mem::replace(&mut self.rows[r][c], value);
                if remeasure {
                    self.measure(c);
                } else {
                    self.widths[c] = self.widths[c].max(self.rows[r][c].width().min(MAX_COL));
                }
                self.cur = (r, c);
                Change::SetCell { r, c, value: old }
            }
            Change::InsertRow { r, row } => {
                for (w, cell) in self.widths.iter_mut().zip(&row) {
                    *w = (*w).max(cell.width().min(MAX_COL));
                }
                self.rows.insert(r, row);
                self.cur.0 = r;
                Change::DeleteRow { r }
            }
            Change::DeleteRow { r } => {
                let row = self.rows.remove(r);
                if remeasure {
                    self.measure_all();
                }
                self.cur.0 = r.min(self.rows.len() - 1);
                Change::InsertRow { r, row }
            }
            Change::InsertCol { c, col } => {
                for (row, cell) in self.rows.iter_mut().zip(col) {
                    row.insert(c, cell);
                }
                self.widths.insert(c, 0);
                self.measure(c);
                self.cur.1 = c;
                Change::DeleteCol { c }
            }
            Change::DeleteCol { c } => {
                let col = self.rows.iter_mut().map(|row| row.remove(c)).collect();
                self.widths.remove(c);
                self.cur.1 = c.min(self.ncols() - 1);
                Change::InsertCol { c, col }
            }
            Change::Batch { at, changes } => {
                let mut undo: Vec<_> = changes.into_iter().map(|ch| self.apply(ch, false)).collect();
                undo.reverse();
                self.measure_all();
                self.cur = (at.0.min(self.rows.len() - 1), at.1.min(self.ncols() - 1));
                Change::Batch { at, changes: undo }
            }
        }
    }

    /// Apply a new edit and make it undoable.
    fn record(&mut self, ch: Change) {
        let change = self.exec(ch);
        self.refilter();
        self.next_id += 1;
        self.undo.push(Entry { id: self.next_id, change });
        self.redo.clear();
    }

    fn undo(&mut self) {
        let Some(Entry { id, change }) = self.undo.pop() else {
            self.status = "already at oldest change".into();
            return;
        };
        let change = self.exec(change);
        self.refilter();
        self.redo.push(Entry { id, change });
        self.status = format!("undone — {} more, ^R to redo", self.undo.len());
    }

    fn redo(&mut self) {
        let Some(Entry { id, change }) = self.redo.pop() else {
            self.status = "already at newest change".into();
            return;
        };
        let change = self.exec(change);
        self.refilter();
        self.undo.push(Entry { id, change });
        self.status = format!("redone — {} more", self.redo.len());
    }

    fn add_column(&mut self) {
        let c = self.cur.1 + 1;
        let col = vec![String::new(); self.rows.len()];
        self.record(Change::InsertCol { c, col });
        self.status = format!("added column {} — u to undo", c + 1);
    }

    fn delete_row(&mut self) {
        let r = self.cur.0;
        if r == 0 {
            self.status = "can't delete the header row".into();
            return;
        }
        self.record(Change::DeleteRow { r });
        self.status = format!("deleted row {r} — u to undo");
    }

    fn delete_column(&mut self) {
        if self.ncols() == 1 {
            self.status = "can't delete the only column".into();
            return;
        }
        let c = self.cur.1;
        let name = self.rows[0][c].clone();
        self.record(Change::DeleteCol { c });
        self.status = if name.is_empty() {
            format!("deleted column {} — u to undo", c + 1)
        } else {
            format!("deleted column {} ({name}) — u to undo", c + 1)
        };
    }

    fn paste_clipboard(&mut self) {
        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(text) => self.paste(&text),
            Err(e) => self.status = format!("can't read the clipboard ({e}) — paste with your terminal instead"),
        }
    }

    /// Paste TSV text over the grid with its top-left cell at the cursor,
    /// adding rows and columns as needed. One undo step.
    fn paste(&mut self, text: &str) {
        let text = text.strip_suffix('\n').unwrap_or(text);
        let text = text.strip_suffix('\r').unwrap_or(text);
        if text.is_empty() {
            self.status = "nothing to paste".into();
            return;
        }
        let block: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
        let height = block.len();
        let width = block.iter().map(Vec::len).max().unwrap_or(1);
        let (r0, c0) = self.cur;
        // Hidden rows would be overwritten without you seeing it.
        if height > 1 && self.filter.is_some() {
            self.status = "clear the filter (Esc) to paste more than one row".into();
            return;
        }
        let mut changes = Vec::new();
        for c in self.ncols()..c0 + width {
            changes.push(Change::InsertCol { c, col: vec![String::new(); self.rows.len()] });
        }
        let ncols = self.ncols().max(c0 + width);
        for r in self.rows.len()..r0 + height {
            changes.push(Change::InsertRow { r, row: vec![String::new(); ncols] });
        }
        for (dr, line) in block.iter().enumerate() {
            for (dc, cell) in line.iter().enumerate() {
                let (r, c) = (r0 + dr, c0 + dc);
                if self.rows.get(r).and_then(|row| row.get(c)).map(String::as_str) != Some(*cell) {
                    changes.push(Change::SetCell { r, c, value: (*cell).to_owned() });
                }
            }
        }
        if changes.is_empty() {
            self.status = "pasted — no changes".into();
            return;
        }
        self.record(Change::Batch { at: (r0, c0), changes });
        self.status = format!("pasted {height} × {width} — u to undo");
    }

    fn begin_edit(&mut self, at_end: bool) {
        self.edit = self.rows[self.cur.0][self.cur.1].clone();
        self.caret = if at_end { self.edit.len() } else { 0 };
        self.mode = Mode::Insert;
    }

    /// Write the edit buffer back into the grid and leave insert mode.
    fn commit(&mut self, then_down: i32) {
        // Tabs and newlines would corrupt the file format.
        let value = std::mem::take(&mut self.edit).replace(['\t', '\n', '\r'], " ");
        let (r, c) = self.cur;
        if value != self.rows[r][c] {
            self.record(Change::SetCell { r, c, value });
        }
        self.mode = Mode::Normal;
        self.caret = 0;
        if then_down != 0 {
            self.move_to(then_down, 0);
        }
    }

    fn move_to(&mut self, dr: i32, dc: i32) {
        let c = (self.cur.1 as i32 + dc).clamp(0, self.ncols() as i32 - 1);
        self.cur.1 = c as usize;
        // While filtering, step through the visible rows (header first).
        let rows = match self.visible() {
            Some(v) => [0].into_iter().chain(v).collect(),
            None => (0..self.rows.len()).collect::<Vec<_>>(),
        };
        let i = rows.binary_search(&self.cur.0).unwrap_or_else(|i| i);
        self.cur.0 = rows[(i as i32 + dr).clamp(0, rows.len() as i32 - 1) as usize];
    }

    // --------------------------------------------------------------- render

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        if area.height < 3 || area.width < 8 {
            return;
        }
        let body_h = area.height as usize - 2; // header + status
        let gutter = self.rows.len().to_string().len().max(2);

        // Widen the active column while editing so the caret stays visible.
        let widths: Vec<usize> = (0..self.ncols())
            .map(|i| {
                if self.mode == Mode::Insert && i == self.cur.1 {
                    self.widths[i].max(self.edit.width() + 1)
                } else {
                    self.widths[i]
                }
            })
            .collect();

        // Vertical scrolling over data rows only (row 0 is a pinned header).
        let vis = self.visible();
        let n = vis.as_ref().map_or(self.rows.len() - 1, Vec::len);
        let row_at = |i: usize| vis.as_ref().map_or(i + 1, |v| v[i]);
        // On the header, scroll to the first data row.
        let d = match &vis {
            _ if self.cur.0 == 0 => 0,
            Some(v) => v.binary_search(&self.cur.0).unwrap_or_else(|i| i),
            None => self.cur.0 - 1,
        };
        self.off.0 = self.off.0.min(d);
        if d >= self.off.0 + body_h {
            self.off.0 = d + 1 - body_h;
        }

        // Horizontal scrolling: advance the offset until the cursor column fits.
        let avail = area.width as usize - gutter - 1;
        self.off.1 = self.off.1.min(self.cur.1);
        let mut cols: Vec<usize> = Vec::new();
        loop {
            cols.clear();
            let mut used = 0;
            for i in self.off.1..self.ncols() {
                let w = widths[i] + 1;
                if used + w > avail && i > self.off.1 {
                    break;
                }
                used += w;
                cols.push(i);
            }
            if cols.contains(&self.cur.1) || self.off.1 >= self.cur.1 {
                break;
            }
            self.off.1 += 1;
        }

        let mut lines = Vec::with_capacity(body_h + 1);
        lines.push(self.line(0, &cols, &widths, gutter));
        self.off.0 = self.off.0.min(n.saturating_sub(body_h));
        for i in self.off.0..n.min(self.off.0 + body_h) {
            lines.push(self.line(row_at(i), &cols, &widths, gutter));
        }

        f.render_widget(Paragraph::new(lines), area);
        f.render_widget(self.status_line(), Rect::new(area.x, area.bottom() - 1, area.width, 1));
    }

    fn line(&self, r: usize, cols: &[usize], widths: &[usize], gutter: usize) -> Line<'static> {
        let header = r == 0;
        let active_row = r == self.cur.0;
        // Only highlight hits in rows that actually match.
        let filter = self
            .filter
            .as_ref()
            .filter(|f| !header && !f.terms.is_empty() && f.rows.binary_search(&r).is_ok());
        let mut spans = Vec::with_capacity(cols.len() * 2 + 1);
        spans.push(Span::styled(
            format!("{:>gutter$} ", if header { "#".into() } else { r.to_string() }),
            Style::new().fg(Color::DarkGray),
        ));

        for &c in cols {
            let w = widths[c];
            let active = active_row && c == self.cur.1;
            let mut style = Style::new().fg(PALETTE[c % PALETTE.len()]);
            if header {
                style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
            }
            if active_row && !active {
                style = style.bg(Color::Indexed(236));
            }

            if active && self.mode == Mode::Insert {
                let (a, b) = self.edit.split_at(self.caret);
                let mut caret = b.chars().next().map(String::from).unwrap_or_else(|| " ".into());
                let rest = &b[caret.len().min(b.len())..];
                if caret == "\t" {
                    caret = " ".into();
                }
                let used = a.width() + caret.width() + rest.width();
                let base = Style::new().fg(Color::Black).bg(Color::LightYellow);
                spans.push(Span::styled(a.to_owned(), base));
                spans.push(Span::styled(caret, base.add_modifier(Modifier::REVERSED)));
                spans.push(Span::styled(rest.to_owned(), base));
                spans.push(Span::styled(" ".repeat(w.saturating_sub(used)), base));
            } else {
                if active {
                    style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
                }
                let cell = &self.rows[r][c];
                let mut text = fit(cell, w);
                if filter.is_some_and(|f| f.cell_hit(cell)) {
                    // Underline just the text, not the column padding.
                    let pad = text.split_off(text.trim_end_matches(' ').len());
                    spans.push(Span::styled(
                        text,
                        style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                    ));
                    spans.push(Span::styled(pad, style));
                } else {
                    spans.push(Span::styled(text, style));
                }
            }
            spans.push(Span::raw(" "));
        }
        Line::from(spans)
    }

    fn status_line(&self) -> Paragraph<'static> {
        if self.mode == Mode::Command {
            return Paragraph::new(self.cmd.clone());
        }
        let count = |f: &Filter| format!("{} of {} rows", f.rows.len(), self.rows.len() - 1);
        if let (Mode::Filter, Some(f)) = (self.mode, &self.filter) {
            return Paragraph::new(Line::from(vec![
                Span::raw(format!("/{}", f.query)),
                Span::styled(" ", Style::new().add_modifier(Modifier::REVERSED)),
                Span::styled(
                    format!("  {} — Enter keeps, Esc clears", count(f)),
                    Style::new().fg(Color::DarkGray),
                ),
            ]));
        }
        let (tag, color) = match self.mode {
            Mode::Insert => (" INSERT ", Color::LightGreen),
            _ => (" NORMAL ", Color::LightBlue),
        };
        let name = self.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let head = self.rows[0].get(self.cur.1).cloned().unwrap_or_default();
        Paragraph::new(Line::from(vec![
            Span::styled(tag, Style::new().fg(Color::Black).bg(color).bold()),
            Span::styled(
                format!(
                    " {name}{}  {}:{} [{}] {}{} ",
                    if self.dirty() { " *" } else { "" },
                    self.cur.0,
                    self.cur.1 + 1,
                    if head.is_empty() { "—".into() } else { head },
                    match self.pending {
                        Some(Pending::T) => "t",
                        Some(Pending::D) => "d",
                        Some(Pending::X { .. }) => "x",
                        None => "",
                    },
                    self.filter
                        .as_ref()
                        .map_or(String::new(), |f| format!(" /{} ({})", f.query, count(f)))
                ),
                Style::new().fg(Color::Gray),
            ),
            Span::styled(self.status.clone(), Style::new().fg(Color::DarkGray)),
        ]))
    }
}

/// Pad or ellipsize `s` to exactly `w` display columns.
fn fit(s: &str, w: usize) -> String {
    let sw = s.width();
    if sw <= w {
        let mut out = String::with_capacity(s.len() + w - sw);
        out.push_str(s);
        out.push_str(&" ".repeat(w - sw));
        return out;
    }
    let mut out = String::with_capacity(w);
    let mut used = 0;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if used + cw > w - 1 {
            break;
        }
        out.push(ch);
        used += cw;
    }
    out.push('…');
    out.push_str(&" ".repeat(w - used - 1));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(tsv: &str) -> App {
        let dir = env::temp_dir().join(format!("tsv-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("{n}.tsv"));
        fs::write(&path, tsv).unwrap();
        App::load(path).unwrap()
    }

    fn keys(app: &mut App, s: &str) {
        for ch in s.chars() {
            let code = match ch {
                '\x1b' => KeyCode::Esc,
                '\n' => KeyCode::Enter,
                '\t' => KeyCode::Tab,
                c => KeyCode::Char(c),
            };
            app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        }
    }

    fn ctrl_r(app: &mut App) {
        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    }

    fn grid(app: &App) -> String {
        app.rows.iter().map(|r| r.join("\t")).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn tt_adds_column_right_of_cursor() {
        let mut a = app("a\tb\n1\t2\n");
        keys(&mut a, "tt");
        assert_eq!(grid(&a), "a\t\tb\n1\t\t2");
        assert_eq!(a.cur, (0, 1));
        assert_eq!(a.widths.len(), 3);
        keys(&mut a, "u");
        assert_eq!(grid(&a), "a\tb\n1\t2");
    }

    #[test]
    fn xx_deletes_column_and_undo_restores_it_in_one_step() {
        let mut a = app("a\tb\tc\n1\t2\t3\n");
        keys(&mut a, "lxx");
        assert_eq!(grid(&a), "a\tc\n1\t3");
        keys(&mut a, "u");
        assert_eq!(grid(&a), "a\tb\tc\n1\t2\t3");
        assert!(!a.dirty());
        ctrl_r(&mut a);
        assert_eq!(grid(&a), "a\tc\n1\t3");
    }

    #[test]
    fn single_x_still_clears_the_cell() {
        let mut a = app("a\tb\n1\t2\n");
        keys(&mut a, "jxl");
        assert_eq!(grid(&a), "a\tb\n\t2");
        assert_eq!(a.cur, (1, 1));
    }

    #[test]
    fn xx_refuses_the_only_column() {
        let mut a = app("a\n1\n");
        keys(&mut a, "xx");
        assert_eq!(grid(&a), "a\n1");
    }

    #[test]
    fn undo_redo_cells_and_rows() {
        let mut a = app("a\tb\n1\t2\n");
        keys(&mut a, "jihi\x1bo");
        assert_eq!(grid(&a), "a\tb\nhi1\t2\n\t");
        keys(&mut a, "uu");
        assert_eq!(grid(&a), "a\tb\n1\t2");
        keys(&mut a, "u");
        assert_eq!(a.status, "already at oldest change");
        ctrl_r(&mut a);
        ctrl_r(&mut a);
        assert_eq!(grid(&a), "a\tb\nhi1\t2\n\t");
        keys(&mut a, "D");
        keys(&mut a, "u");
        assert_eq!(grid(&a), "a\tb\nhi1\t2\n\t");
    }

    #[test]
    fn dirty_follows_undo_back_to_saved_state() {
        let mut a = app("a\tb\n");
        keys(&mut a, "tt");
        assert!(a.dirty());
        a.save();
        assert!(!a.dirty());
        keys(&mut a, "u");
        assert!(a.dirty());
        ctrl_r(&mut a);
        assert!(!a.dirty());
        let _ = fs::remove_file(&a.path);
    }

    const PEOPLE: &str = "name\tcity\n\
                          ann\tZürich\n\
                          bob\tOslo\n\
                          cat\tzürich\n\
                          dan\tBern\n";

    #[test]
    fn dd_deletes_the_row_but_not_the_header() {
        let mut a = app("h\n1\n2\n");
        keys(&mut a, "jdd");
        assert_eq!(grid(&a), "h\n2");
        keys(&mut a, "gdd");
        assert_eq!(grid(&a), "h\n2");
        keys(&mut a, "jdjd");
        assert_eq!(grid(&a), "h\n2", "d then another key is not a delete");
        keys(&mut a, "u");
        assert_eq!(grid(&a), "h\n1\n2");
    }

    #[test]
    fn tab_on_the_last_column_adds_one() {
        let mut a = app("a\tb\n1\t2\n");
        keys(&mut a, "j$ix\ty\x1b");
        assert_eq!(grid(&a), "a\tb\t\n1\tx2\ty");
        assert_eq!(a.cur, (1, 2));
        keys(&mut a, "uu");
        assert_eq!(grid(&a), "a\tb\n1\tx2");
    }

    #[test]
    fn colon_number_jumps_to_row() {
        let mut a = app(PEOPLE);
        keys(&mut a, ":3\n");
        assert_eq!(a.cur.0, 3);
        keys(&mut a, ":100\n");
        assert_eq!(a.cur.0, 4, "clamped to the last row");
        keys(&mut a, ":0\n");
        assert_eq!(a.cur.0, 0);
        keys(&mut a, ":$\n");
        assert_eq!(a.cur.0, 4);
        keys(&mut a, ":3x\n");
        assert_eq!(a.status, "unknown command: :3x");
    }

    /// Render into an in-memory terminal and return the first cell of each
    /// line below the header.
    fn first_column(app: &mut App, height: u16) -> Vec<String> {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, height)).unwrap();
        let buf = term.draw(|f| app.draw(f)).unwrap().buffer.clone();
        (1..height - 1)
            .map(|y| (0..40).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .map(|s| s.split_whitespace().nth(1).unwrap_or("").to_owned())
            .collect()
    }

    #[test]
    fn g_scrolls_back_to_the_first_row() {
        let tsv: String = (0..100).map(|i| format!("r{i}\n")).collect();
        let mut a = app(&tsv);
        keys(&mut a, "G");
        assert_eq!(first_column(&mut a, 7).last().unwrap(), "r99");
        keys(&mut a, "g");
        assert_eq!(first_column(&mut a, 7), ["r1", "r2", "r3", "r4", "r5"]);
        keys(&mut a, "G:0\n");
        assert_eq!(first_column(&mut a, 7)[0], "r1");
    }

    #[test]
    fn tab_keeps_editing_in_the_next_cell() {
        let mut a = app("a\tb\n1\t2\n");
        keys(&mut a, "jix\ty");
        assert!(a.mode == Mode::Insert);
        assert_eq!(a.cur, (1, 1));
        a.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert!(a.mode == Mode::Insert);
        keys(&mut a, "z\x1b");
        assert_eq!(grid(&a), "a\tb\nx1z\t2y");
        keys(&mut a, "u");
        assert_eq!(grid(&a), "a\tb\nx1\t2y", "each cell is its own undo step");
    }

    #[test]
    fn arrows_keep_editing_the_cell_above_and_below() {
        let mut a = app("a\tb\n1\t2\n3\t4\n");
        keys(&mut a, "jix");
        a.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert!(a.mode == Mode::Insert);
        assert_eq!(a.cur, (2, 0));
        keys(&mut a, "y");
        a.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(a.cur, (2, 0), "stays on the last row");
        keys(&mut a, "z");
        a.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        a.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert!(a.mode == Mode::Insert);
        assert_eq!(a.cur, (0, 0));
        keys(&mut a, "!\x1b");
        assert_eq!(grid(&a), "a!\tb\nx1\t2\n3yz\t4");
    }

    #[test]
    fn paste_overwrites_from_the_cursor_and_grows_the_grid() {
        let mut a = app("a\tb\n1\t2\n");
        keys(&mut a, "jl");
        a.on_paste("x\ty\r\nz\r\n");
        assert_eq!(grid(&a), "a\tb\t\n1\tx\ty\n\tz\t");
        assert_eq!(a.cur, (1, 1));
        assert!(a.dirty());
        keys(&mut a, "u");
        assert_eq!(grid(&a), "a\tb\n1\t2", "one undo step");
        assert!(!a.dirty());
        ctrl_r(&mut a);
        assert_eq!(grid(&a), "a\tb\t\n1\tx\ty\n\tz\t");
    }

    #[test]
    fn large_paste_is_fast() {
        let mut a = app("");
        let text: String = (0..20_000).map(|i| format!("{i}\ta\tb\tc\n")).collect();
        let t = std::time::Instant::now();
        a.on_paste(&text);
        keys(&mut a, "u");
        ctrl_r(&mut a);
        assert_eq!(a.rows.len(), 20_000);
        assert!(t.elapsed().as_secs() < 5, "took {:?}", t.elapsed());
    }

    #[test]
    fn paste_into_an_empty_file_fills_it() {
        let mut a = app("");
        a.on_paste("name\tcity\nAna\tBern\n");
        assert_eq!(grid(&a), "name\tcity\nAna\tBern");
    }

    #[test]
    fn paste_while_editing() {
        let mut a = app("a\tb\n1\t2\n");
        keys(&mut a, "ja");
        a.on_paste("23");
        assert!(a.mode == Mode::Insert, "plain text goes in at the caret");
        keys(&mut a, "\x1b");
        assert_eq!(grid(&a), "a\tb\n123\t2");
        keys(&mut a, "i");
        a.on_paste("x\ty");
        assert!(a.mode == Mode::Normal, "a block commits the cell and pastes");
        assert_eq!(grid(&a), "a\tb\nx\ty");
    }

    #[test]
    fn multi_row_paste_is_refused_while_filtering() {
        let mut a = app("a\tb\n1\t2\n3\t4\n");
        keys(&mut a, "/3\n");
        a.on_paste("x\ny");
        assert_eq!(grid(&a), "a\tb\n1\t2\n3\t4");
        a.on_paste("x");
        assert_eq!(grid(&a), "a\tb\n1\t2\nx\t4");
    }

    #[test]
    fn filter_keeps_matching_rows_and_marks_hit_cells() {
        let mut a = app(PEOPLE);
        keys(&mut a, "/zür");
        let f = a.filter.as_ref().unwrap();
        assert_eq!(f.rows, [1, 3]);
        assert!(f.cell_hit("Zürich") && !f.cell_hit("ann"));
        assert_eq!(a.cur.0, 1);
        keys(&mut a, "\njjj");
        assert_eq!(a.cur.0, 3, "j skips hidden rows and stops at the last match");
        keys(&mut a, "k");
        assert_eq!(a.cur.0, 1);
    }

    #[test]
    fn filter_terms_and_smartcase() {
        let mut a = app(PEOPLE);
        keys(&mut a, "/Zür");
        assert_eq!(a.filter.as_ref().unwrap().rows, [1]);
        keys(&mut a, "\x1b/zür cat");
        assert_eq!(a.filter.as_ref().unwrap().rows, [3], "every term must match");
    }

    #[test]
    fn esc_clears_filter_and_edits_keep_the_cursor_row() {
        let mut a = app(PEOPLE);
        keys(&mut a, "/oslo\n");
        assert_eq!(a.cur.0, 2);
        keys(&mut a, "lxo");
        assert_eq!(a.filter.as_ref().unwrap().rows, Vec::<usize>::new());
        assert_eq!(a.visible().unwrap(), [3], "the new row stays while the cursor is on it");
        keys(&mut a, "\x1b");
        assert!(a.filter.is_none());
        keys(&mut a, "G");
        assert_eq!(a.cur.0, 5);
    }
}
