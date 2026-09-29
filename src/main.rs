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
    dirty: bool,
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
            dirty: false,
            status: String::new(),
            quit: false,
        };
        for c in 0..ncols {
            app.measure(c);
        }
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

    fn save(&mut self) {
        let mut out = String::with_capacity(self.rows.len() * 32);
        for r in &self.rows {
            out.push_str(&r.join("\t"));
            out.push('\n');
        }
        match fs::write(&self.path, out) {
            Ok(()) => {
                self.dirty = false;
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
        match k.code {
            KeyCode::Char('s') if ctrl => self.save(),
            KeyCode::Char('c') if ctrl => self.quit = true,
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
            KeyCode::Char('x') => {
                self.cell_mut().clear();
                let c = self.cur.1;
                self.measure(c);
                self.dirty = true;
            }
            KeyCode::Char('o') => {
                let r = (self.cur.0 + 1).max(1);
                self.rows.insert(r, vec![String::new(); self.ncols()]);
                self.cur.0 = r;
                self.dirty = true;
            }
            KeyCode::Char('D') => {
                if self.cur.0 > 0 && self.rows.len() > 1 {
                    self.rows.remove(self.cur.0);
                    self.cur.0 = self.cur.0.min(self.rows.len() - 1);
                    for c in 0..self.ncols() {
                        self.measure(c);
                    }
                    self.dirty = true;
                }
            }
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.cmd = ":".into();
            }
            KeyCode::Char('q') => {
                if self.dirty {
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
                        self.quit = !self.dirty;
                    }
                    ":q" => {
                        if self.dirty {
                            self.status = "unsaved changes — :q! to discard".into();
                        } else {
                            self.quit = true;
                        }
                    }
                    ":q!" => self.quit = true,
                    ":h" => {
                        self.status =
                            "arrows/hjkl/Tab move · i,a edit · Esc commit · x clear · o new row \
                             · D delete row · g,G,0,$ jump · :w :q :wq"
                                .into()
                    }
                    other => self.status = format!("unknown command: {other}"),
                }
            }
            _ => {}
        }
    }

    // ----------------------------------------------------------------- edit

    fn cell_mut(&mut self) -> &mut String {
        &mut self.rows[self.cur.0][self.cur.1]
    }

    fn begin_edit(&mut self, at_end: bool) {
        self.edit = self.rows[self.cur.0][self.cur.1].clone();
        self.caret = if at_end { self.edit.len() } else { 0 };
        self.mode = Mode::Insert;
    }

    /// Write the edit buffer back into the grid and leave insert mode.
    fn commit(&mut self, then_down: i32) {
        let value = std::mem::take(&mut self.edit);
        if value != self.rows[self.cur.0][self.cur.1] {
            // Tabs and newlines would corrupt the file format.
            *self.cell_mut() = value.replace(['\t', '\n', '\r'], " ");
            let c = self.cur.1;
            self.measure(c);
            self.dirty = true;
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
                    " {name}{}  {}:{} [{}]  ",
                    if self.dirty { " *" } else { "" },
                    self.cur.0,
                    self.cur.1 + 1,
                    if head.is_empty() { "—".into() } else { head }
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
