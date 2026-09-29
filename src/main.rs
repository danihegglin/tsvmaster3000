//! tsv — a fast TUI viewer/editor for tab-separated files.
//!
//! Usage: tsv <file.tsv>

use std::{env, fs, io, path::PathBuf};

use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
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
}

/// A reversible edit. `App::exec` applies one and returns its inverse, so the
/// undo and redo stacks just hold whatever undoes the last step.
enum Change {
    SetCell { r: usize, c: usize, value: String },
    InsertRow { r: usize, row: Vec<String> },
    DeleteRow { r: usize },
    InsertCol { c: usize, col: Vec<String> },
    DeleteCol { c: usize },
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
    let res = app.run(&mut terminal);
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
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    self.on_key(k);
                }
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
        }
    }

    fn normal(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match (self.pending.take(), k.code) {
            (Some(Pending::T), KeyCode::Char('t')) => return self.add_column(),
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
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char('t') => self.pending = Some(Pending::T),
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
            KeyCode::Char('G') => self.cur.0 = self.rows.len() - 1,
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
            KeyCode::Char('D') => {
                if self.cur.0 > 0 && self.rows.len() > 1 {
                    self.record(Change::DeleteRow { r: self.cur.0 });
                }
            }
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
            KeyCode::Tab => {
                self.commit(0);
                self.move_to(0, 1);
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
                    ":wq" | ":x" => {
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
                             · D delete row · tt/xx add/delete column · u/^R undo/redo \
                             · g,G,0,$ jump · :w :q :wq"
                                .into()
                    }
                    other => self.status = format!("unknown command: {other}"),
                }
            }
            _ => {}
        }
    }

    // ----------------------------------------------------------------- edit

    /// Apply `ch`, put the cursor on it, and return the change that undoes it.
    fn exec(&mut self, ch: Change) -> Change {
        match ch {
            Change::SetCell { r, c, value } => {
                let old = std::mem::replace(&mut self.rows[r][c], value);
                self.measure(c);
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
                self.measure_all();
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
        }
    }

    /// Apply a new edit and make it undoable.
    fn record(&mut self, ch: Change) {
        let change = self.exec(ch);
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
        self.redo.push(Entry { id, change });
        self.status = format!("undone — {} more, ^R to redo", self.undo.len());
    }

    fn redo(&mut self) {
        let Some(Entry { id, change }) = self.redo.pop() else {
            self.status = "already at newest change".into();
            return;
        };
        let change = self.exec(change);
        self.undo.push(Entry { id, change });
        self.status = format!("redone — {} more", self.redo.len());
    }

    fn add_column(&mut self) {
        let c = self.cur.1 + 1;
        let col = vec![String::new(); self.rows.len()];
        self.record(Change::InsertCol { c, col });
        self.status = format!("added column {} — u to undo", c + 1);
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
        let r = (self.cur.0 as i32 + dr).clamp(0, self.rows.len() as i32 - 1);
        let c = (self.cur.1 as i32 + dc).clamp(0, self.ncols() as i32 - 1);
        self.cur = (r as usize, c as usize);
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
        if self.cur.0 >= 1 {
            let d = self.cur.0 - 1;
            self.off.0 = self.off.0.min(d);
            if d >= self.off.0 + body_h {
                self.off.0 = d + 1 - body_h;
            }
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
        for r in (self.off.0 + 1)..self.rows.len().min(self.off.0 + 1 + body_h) {
            lines.push(self.line(r, &cols, &widths, gutter));
        }

        f.render_widget(Paragraph::new(lines), area);
        f.render_widget(self.status_line(), Rect::new(area.x, area.bottom() - 1, area.width, 1));
    }

    fn line(&self, r: usize, cols: &[usize], widths: &[usize], gutter: usize) -> Line<'static> {
        let header = r == 0;
        let active_row = r == self.cur.0;
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
                spans.push(Span::styled(fit(&self.rows[r][c], w), style));
            }
            spans.push(Span::raw(" "));
        }
        Line::from(spans)
    }

    fn status_line(&self) -> Paragraph<'static> {
        if self.mode == Mode::Command {
            return Paragraph::new(self.cmd.clone());
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
                    " {name}{}  {}:{} [{}] {} ",
                    if self.dirty() { " *" } else { "" },
                    self.cur.0,
                    self.cur.1 + 1,
                    if head.is_empty() { "—".into() } else { head },
                    match self.pending {
                        Some(Pending::T) => "t",
                        Some(Pending::X { .. }) => "x",
                        None => "",
                    }
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
}
