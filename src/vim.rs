//! Vim-style key handling: normal, visual, insert and command-line modes.

use std::path::PathBuf;

use gpui::Context;
use regex::Regex;

use crate::editor::{col_name, Editor, Mode, Search, fuzzy_match, is_text};
use crate::lineedit::LineEdit;
use crate::table::{Change, Table};

enum Res {
    Done,
    Pending,
    Invalid,
}

fn is_linewise(key: &str) -> bool {
    matches!(
        key,
        "j" | "k"
            | "G"
            | "g"
            | "H"
            | "M"
            | "L"
            | "{"
            | "}"
            | "n"
            | "N"
            | "+"
            | "-"
            | "<up>"
            | "<down>"
            | "<cr>"
            | "<c-d>"
            | "<c-u>"
            | "<c-f>"
            | "<c-b>"
            | "<pageup>"
            | "<pagedown>"
    )
}

/// Split a leading count off a key sequence.
fn split_count<'a>(keys: &'a [&'a str]) -> (Option<usize>, &'a [&'a str]) {
    let mut n = 0;
    let mut digits = String::new();
    while let Some(&k) = keys.get(n) {
        let is_digit = k.len() == 1 && k.as_bytes()[0].is_ascii_digit();
        if !is_digit || (k == "0" && digits.is_empty()) {
            break;
        }
        digits.push_str(k);
        n += 1;
    }
    (digits.parse().ok(), &keys[n..])
}

/// Compile a search pattern with vim's smartcase; invalid regexes are
/// searched literally.
pub fn compile_search(pattern: &str) -> Regex {
    let icase = !pattern.chars().any(char::is_uppercase);
    let prefix = if icase { "(?i)" } else { "" };
    Regex::new(&format!("{prefix}{pattern}"))
        .or_else(|_| Regex::new(&format!("{prefix}{}", regex::escape(pattern))))
        .expect("escaped pattern is valid")
}

impl Editor {
    pub fn handle_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let table_mode = matches!(self.mode, Mode::Normal | Mode::Visual { .. });
        match key {
            "<d-q>" => return self.ex("q", cx),
            "<d-s>" => return self.ex("w", cx),
            "<d-z>" => {
                self.leave_mode();
                return self.undo(false);
            }
            "<d-s-z>" | "<d-y>" => {
                self.leave_mode();
                return self.undo(true);
            }
            "<d-enter>" => return self.add_row(self.row + 1),
            "<d-s-enter>" => return self.add_row(self.row),
            // Rows and columns: ⌥+arrow adds in that direction, ⌘⌫ / ⌘⇧⌫
            // delete rows / columns (the selection in visual mode). In insert
            // mode these keys keep their text-editing meaning.
            "<a-right>" if table_mode => return self.add_col(self.col + 1),
            "<a-left>" if table_mode => return self.add_col(self.col),
            "<a-down>" if table_mode => return self.add_row(self.row + 1),
            "<a-up>" if table_mode => return self.add_row(self.row),
            "<d-backspace>" if table_mode => return self.remove_rows(),
            "<d-s-backspace>" if table_mode => return self.remove_cols(),
            // ⌘+ / ⌘− add a column right of the cursor / delete the cursor's
            // (or the selected) columns, in any mode.
            "<d-=>" | "<d-+>" | "<d-s-=>" | "<d-s-+>" => return self.add_col(self.col + 1),
            "<d-->" => return self.remove_cols(),
            _ => {}
        }
        match self.mode {
            Mode::Normal => self.normal_key(key, cx),
            Mode::Visual { line } => self.visual_key(key, line, cx),
            Mode::Insert => self.insert_key(key, cx),
            Mode::Command | Mode::Search { .. } => self.cmdline_key(key, cx),
            Mode::Filter => self.filter_key(key, cx),
        }
    }

    // ---- normal mode -----------------------------------------------------

    fn normal_key(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.pending.is_empty() {
            self.message = None;
        }
        self.pending.push(key.to_owned());
        if !self.dot.replaying {
            self.dot.rec.push(key.to_owned());
        }
        let pending = self.pending.clone();
        let keys: Vec<&str> = pending.iter().map(String::as_str).collect();
        let (count, rest) = split_count(&keys);

        if rest == ["."] {
            self.pending.clear();
            self.dot.rec.clear();
            let last = self.dot.last.clone();
            if last.is_empty() {
                return;
            }
            self.dot.replaying = true;
            for _ in 0..count.unwrap_or(1) {
                for k in &last {
                    self.handle_key(k, cx);
                }
            }
            self.dot.replaying = false;
            self.commit();
            return;
        }

        // `xx`: the first `x` already cleared the cell; put it back and
        // delete the whole column instead, as one undo step.
        let after_x = self.after_x.take();
        if rest == ["x"] && count.is_none() && !self.dot.replaying
            && let Some(changed) = after_x
        {
            self.pending.clear();
            self.dot.rec.clear();
            if changed {
                self.undo(false);
            }
            return self.remove_cols();
        }

        let res = self.normal_cmd(rest, count, cx);
        if let Res::Pending = res {
            return;
        }
        if rest == ["x"] && !self.dot.replaying {
            self.after_x = Some(!self.history.cur.is_empty());
        }
        self.pending.clear();
        if self.dot.replaying {
            return;
        }
        match (res, self.mode) {
            (Res::Done, Mode::Insert) => self.dot.in_insert = true,
            (Res::Done, _) => {
                if !self.history.cur.is_empty() {
                    self.dot.last = std::mem::take(&mut self.dot.rec);
                }
                self.dot.rec.clear();
                self.commit();
            }
            _ => self.dot.rec.clear(),
        }
    }

    fn normal_cmd(&mut self, keys: &[&str], count: Option<usize>, cx: &mut Context<Self>) -> Res {
        let n = count.unwrap_or(1);
        if let Some(res) = self.motion(keys, count) {
            return res;
        }
        match keys {
            [] => Res::Pending,
            ["<esc>"] | ["<c-[>"] | ["<c-c>"] => {
                self.set_filter(String::new());
                Res::Done
            }
            [" "] => Res::Pending,
            [" ", " "] => {
                self.cmdline = LineEdit::new(self.filter.clone(), true);
                self.mode = Mode::Filter;
                Res::Done
            }

            // Operators.
            ["d" | "y" | "c", ..] => self.operator(keys, count),

            ["x"] | ["<del>"] => {
                let c1 = (self.col + n - 1).min(self.last_col());
                self.yank_block(self.row, self.row, self.col, c1);
                self.clear_block(self.row, self.row, self.col, c1);
                Res::Done
            }
            ["D"] => {
                let c1 = self.last_col();
                self.yank_block(self.row, self.row, self.col, c1);
                self.clear_block(self.row, self.row, self.col, c1);
                Res::Done
            }
            ["s"] | ["S"] | ["C"] => {
                self.yank_block(self.row, self.row, self.col, self.col);
                self.set_cell(self.row, self.col, String::new());
                self.begin_insert(true);
                Res::Done
            }
            ["i"] => {
                self.begin_insert(false);
                Res::Done
            }
            ["a"] | ["<cr>"] | ["<f2>"] => {
                self.begin_insert(true);
                Res::Done
            }
            ["I"] => {
                self.col = 0;
                self.begin_insert(false);
                Res::Done
            }
            ["A"] => {
                self.col = self.table.row_len(self.row).saturating_sub(1);
                self.begin_insert(true);
                Res::Done
            }
            ["o"] | ["O"] => {
                let at = if keys[0] == "o" { self.row + 1 } else { self.row };
                self.insert_rows(at, vec![vec![String::new()]; n]);
                self.row = at;
                self.col = 0;
                self.begin_insert(true);
                Res::Done
            }
            ["p"] | ["P"] => {
                for _ in 0..n {
                    self.paste(keys[0] == "P", cx);
                }
                Res::Done
            }
            ["u"] => {
                for _ in 0..n {
                    self.undo(false);
                }
                Res::Done
            }
            ["<c-r>"] => {
                for _ in 0..n {
                    self.undo(true);
                }
                Res::Done
            }
            ["~"] => {
                let c1 = (self.col + n - 1).min(self.last_col());
                self.map_block((self.row, self.row, self.col, c1), |s| Some(toggle_case(s)));
                self.col = (c1 + 1).min(self.last_col());
                Res::Done
            }
            ["<c-a>"] | ["<c-x>"] => {
                let delta = if keys[0] == "<c-a>" { n as i64 } else { -(n as i64) };
                self.map_block((self.row, self.row, self.col, self.col), |s| {
                    increment(s, delta)
                });
                Res::Done
            }
            [">"] => {
                self.resize_col(self.col, 2 * n as i32);
                Res::Done
            }
            ["<"] => {
                self.resize_col(self.col, -2 * n as i32);
                Res::Done
            }
            ["="] => {
                self.fit_col(self.col);
                Res::Done
            }
            ["v"] | ["<c-v>"] | ["V"] => {
                self.anchor = (self.row, self.col);
                self.mode = Mode::Visual { line: keys[0] == "V" };
                Res::Done
            }
            [":"] => {
                self.cmdline.clear();
                if let Some(c) = count {
                    self.cmdline.insert(&format!(".,.+{}", c - 1));
                }
                self.cmd_history_pos = None;
                self.mode = Mode::Command;
                Res::Done
            }
            ["/"] | ["?"] => {
                self.cmdline.clear();
                self.cmd_history_pos = None;
                self.mode = Mode::Search { forward: keys[0] == "/" };
                Res::Done
            }
            ["*"] | ["#"] => {
                let pattern = format!("^{}$", regex::escape(self.cell()));
                self.search = Some(Search {
                    re: Regex::new(&pattern).unwrap(),
                    pattern,
                    forward: keys[0] == "*",
                });
                self.highlight = true;
                for _ in 0..n {
                    self.search_next(false);
                }
                Res::Done
            }
            ["t"] => Res::Pending,
            ["t", "t"] => {
                let at = self.col + 1;
                for _ in 0..n {
                    self.insert_col(at);
                }
                self.col = at;
                let what = if n == 1 { "column" } else { "columns" };
                self.set_msg(format!("Added {n} {what} at {} · ⌘Z to undo, xx to delete", col_name(at)));
                Res::Done
            }
            ["z"] | ["Z"] | ["g"] => Res::Pending,
            ["z", "z" | "."] => {
                self.top = self.row.saturating_sub(self.page_rows() / 2);
                self.follow_top();
                Res::Done
            }
            ["z", "t" | "<cr>"] => {
                self.top = self.row;
                self.follow_top();
                Res::Done
            }
            ["z", "b" | "-"] => {
                self.top = (self.row + 1).saturating_sub(self.page_rows());
                self.follow_top();
                Res::Done
            }
            ["Z", "Z"] => {
                self.ex("x", cx);
                Res::Done
            }
            ["Z", "Q"] => {
                self.ex("q!", cx);
                Res::Done
            }
            ["g", "v"] => {
                self.mode = Mode::Visual { line: false };
                Res::Done
            }
            ["<c-g>"] => {
                self.file_info();
                Res::Done
            }
            ["<c-l>"] => {
                self.message = None;
                Res::Done
            }
            _ => Res::Invalid,
        }
    }

    /// Scroll set explicitly: clamp, then keep the cursor where it is.
    fn follow_top(&mut self) {
        self.scroll_by(0);
        self.cursor_into_view();
    }

    fn operator(&mut self, keys: &[&str], count: Option<usize>) -> Res {
        let op = keys[0];
        let (count2, motion) = split_count(&keys[1..]);
        let total = match (count, count2) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(1) * b.unwrap_or(1)),
        };
        let n = total.unwrap_or(1);
        match motion {
            [] => return Res::Pending,
            [m] if *m == op => {
                // dd / yy / cc
                let r1 = (self.row + n - 1).min(self.last_row());
                match op {
                    "d" => self.delete_rows(self.row, r1),
                    "y" => {
                        self.yank_rows(self.row, r1);
                        self.set_msg(format!("{} rows yanked", r1 - self.row + 1));
                    }
                    _ => {
                        self.yank_block(self.row, self.row, self.col, self.col);
                        self.set_cell(self.row, self.col, String::new());
                        self.begin_insert(true);
                    }
                }
                return Res::Done;
            }
            ["c"] if op != "c" => {
                // dc / yc: whole columns
                let c1 = (self.col + n - 1).min(self.last_col());
                if op == "d" {
                    self.delete_cols(self.col, n);
                } else {
                    self.yank_cols(self.col, c1);
                    self.set_msg(format!("{} columns yanked", c1 - self.col + 1));
                }
                return Res::Done;
            }
            _ => {}
        }
        let start = (self.row, self.col);
        let res = self.motion(motion, total);
        let end = (self.row, self.col);
        (self.row, self.col) = start;
        match res {
            None | Some(Res::Invalid) => Res::Invalid,
            Some(Res::Pending) => Res::Pending,
            Some(Res::Done) => {
                let (r0, r1) = (start.0.min(end.0), start.0.max(end.0));
                let (c0, c1) = (start.1.min(end.1), start.1.max(end.1));
                if is_linewise(motion[0]) {
                    match op {
                        "d" => self.delete_rows(r0, r1),
                        "y" => {
                            self.yank_rows(r0, r1);
                            self.row = r0;
                        }
                        _ => return Res::Invalid,
                    }
                } else {
                    self.yank_block(self.row, self.row, c0, c1);
                    self.col = c0;
                    if op != "y" {
                        self.clear_block(self.row, self.row, c0, c1);
                    }
                    if op == "c" {
                        self.begin_insert(true);
                    }
                }
                Res::Done
            }
        }
    }

    /// Cursor motions shared by normal and visual mode. `None` if `keys`
    /// is not a motion.
    fn motion(&mut self, keys: &[&str], count: Option<usize>) -> Option<Res> {
        let n = count.unwrap_or(1);
        let last_row = self.last_row();
        let last_col = self.last_col();
        let page = self.page_rows();
        match keys {
            ["h"] | ["<left>"] | ["<bs>"] | ["<s-tab>"] => self.col = self.col.saturating_sub(n),
            ["l"] | ["<right>"] | ["<tab>"] => self.col = (self.col + n).min(last_col),
            ["j"] | ["<down>"] | ["<c-n>"] | ["<c-j>"] | ["+"] => {
                self.row = self.move_rows(self.row, n as isize)
            }
            ["k"] | ["<up>"] | ["<c-p>"] | ["-"] => self.row = self.move_rows(self.row, -(n as isize)),
            ["0"] | ["^"] | ["<home>"] => self.col = 0,
            ["$"] | ["<end>"] => {
                self.col = self.table.row_len(self.row).saturating_sub(1).min(last_col)
            }
            ["|"] => self.col = (n - 1).min(last_col),
            ["w"] | ["W"] | ["e"] | ["E"] => {
                for _ in 0..n {
                    let len = self.table.row_len(self.row);
                    let next = (self.col + 1..len).find(|&c| !self.table.cell(self.row, c).is_empty());
                    self.col = next.unwrap_or(last_col.min(len.max(self.col + 1) - 1));
                }
            }
            ["b"] | ["B"] => {
                for _ in 0..n {
                    let prev = (0..self.col).rev().find(|&c| !self.table.cell(self.row, c).is_empty());
                    self.col = prev.unwrap_or(0);
                }
            }
            ["g"] => return Some(Res::Pending),
            ["g", "g"] => self.row = count.map_or(0, |c| c - 1).min(last_row),
            ["g", "_"] => {
                self.col = self.table.row_len(self.row).saturating_sub(1).min(last_col)
            }
            ["g", "0"] => self.col = self.left,
            ["G"] => self.row = count.map_or(last_row, |c| c - 1).min(last_row),
            ["H"] => self.row = self.move_rows(self.top, n as isize - 1),
            ["M"] => self.row = self.move_rows(self.top, (page / 2) as isize),
            ["L"] => self.row = self.move_rows(self.top, page as isize - n as isize),
            ["<c-d>"] | ["<c-u>"] | ["<c-f>"] | ["<c-b>"] | ["<pagedown>"] | ["<pageup>"] => {
                let amount = match keys[0] {
                    "<c-d>" | "<c-u>" => count.unwrap_or((page / 2).max(1)),
                    _ => n * page.saturating_sub(2).max(1),
                };
                let down = matches!(keys[0], "<c-d>" | "<c-f>" | "<pagedown>");
                let delta = if down { amount as isize } else { -(amount as isize) };
                self.row = self.move_rows(self.row, delta);
                self.scroll_by(delta);
            }
            ["<c-e>"] | ["<c-y>"] => {
                self.scroll_by(if keys[0] == "<c-e>" { n as isize } else { -(n as isize) });
                self.cursor_into_view();
            }
            ["}"] | ["{"] => {
                for _ in 0..n {
                    self.row = self.block_jump(keys[0] == "}");
                }
            }
            ["n"] | ["N"] => {
                let filtering = !self.filter.trim().is_empty();
                for _ in 0..n {
                    if filtering {
                        self.filter_next(keys[0] == "n");
                    } else {
                        self.search_next(keys[0] == "N");
                    }
                }
            }
            _ => return None,
        }
        Some(Res::Done)
    }

    /// Excel-style ctrl-arrow within the current column: to the end of the
    /// current run of non-empty cells, or to the next non-empty cell.
    fn block_jump(&self, down: bool) -> usize {
        let empty = |r: usize| self.table.cell(r, self.col).is_empty();
        let step = |r: usize| -> Option<usize> {
            let next = self.move_rows(r, if down { 1 } else { -1 });
            (next != r).then_some(next)
        };
        let mut r = self.row;
        let Some(next) = step(r) else { return r };
        if !empty(r) && !empty(next) {
            while let Some(nx) = step(r) {
                if empty(nx) {
                    break;
                }
                r = nx;
            }
        } else {
            r = next;
            while empty(r) {
                match step(r) {
                    Some(nx) => r = nx,
                    None => break,
                }
            }
        }
        r
    }

    // ---- visual mode -------------------------------------------------------

    fn visual_key(&mut self, key: &str, line: bool, cx: &mut Context<Self>) {
        self.pending.push(key.to_owned());
        let pending = self.pending.clone();
        let keys: Vec<&str> = pending.iter().map(String::as_str).collect();
        let (count, rest) = split_count(&keys);
        let n = count.unwrap_or(1);
        let sel = self.selection();
        let (r0, r1, c0, c1) = sel;
        let res = match rest {
            [] => Res::Pending,
            ["<esc>"] | ["<c-[>"] | ["<c-c>"] => {
                self.mode = Mode::Normal;
                Res::Done
            }
            ["v"] | ["<c-v>"] | ["V"] => {
                let want_line = rest[0] == "V";
                self.mode = if want_line == line {
                    Mode::Normal
                } else {
                    Mode::Visual { line: want_line }
                };
                Res::Done
            }
            ["o"] | ["O"] => {
                let anchor = self.anchor;
                self.anchor = (self.row, self.col);
                (self.row, self.col) = anchor;
                Res::Done
            }
            ["d"] | ["x"] | ["<del>"] | ["D"] | ["X"] => {
                if line || matches!(rest[0], "D" | "X") {
                    self.delete_rows(r0, r1);
                } else {
                    self.yank_block(r0, r1, c0, c1);
                    self.clear_block(r0, r1, c0, c1);
                    (self.row, self.col) = (r0, c0);
                }
                self.mode = Mode::Normal;
                Res::Done
            }
            ["y"] | ["Y"] => {
                if line || rest[0] == "Y" {
                    self.yank_rows(r0, r1);
                } else {
                    self.yank_block(r0, r1, c0, c1);
                }
                (self.row, self.col) = (r0, c0);
                self.set_msg(format!("{}×{} yanked", r1 - r0 + 1, c1 - c0 + 1));
                self.mode = Mode::Normal;
                Res::Done
            }
            ["c"] | ["s"] => {
                self.yank_block(r0, r1, c0, c1);
                self.clear_block(r0, r1, c0, c1);
                (self.row, self.col) = (r0, c0);
                self.begin_insert(true);
                Res::Done
            }
            ["p"] | ["P"] => {
                (self.row, self.col) = (r0, c0);
                self.mode = Mode::Normal;
                self.paste(false, cx);
                Res::Done
            }
            ["~"] | ["u"] | ["U"] => {
                let f: fn(&str) -> String = match rest[0] {
                    "~" => toggle_case,
                    "u" => |s: &str| s.to_lowercase(),
                    _ => |s: &str| s.to_uppercase(),
                };
                self.map_block(sel, |s| Some(f(s)));
                self.mode = Mode::Normal;
                Res::Done
            }
            ["<c-a>"] | ["<c-x>"] => {
                let delta = if rest[0] == "<c-a>" { n as i64 } else { -(n as i64) };
                self.map_block(sel, |s| increment(s, delta));
                self.mode = Mode::Normal;
                Res::Done
            }
            [">"] | ["<"] | ["="] => {
                for c in c0..=c1 {
                    match rest[0] {
                        ">" => self.resize_col(c, 2 * n as i32),
                        "<" => self.resize_col(c, -2 * n as i32),
                        _ => self.fit_col(c),
                    }
                }
                Res::Done
            }
            [":"] => {
                self.cmdline = LineEdit::new(format!("{},{}", r0 + 1, r1 + 1), true);
                self.cmd_history_pos = None;
                self.mode = Mode::Command;
                Res::Done
            }
            _ => self.motion(rest, count).unwrap_or(Res::Invalid),
        };
        if !matches!(res, Res::Pending) {
            self.pending.clear();
            if self.mode != Mode::Insert {
                self.commit();
            }
        }
    }

    // ---- insert mode -------------------------------------------------------

    fn insert_key(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.dot.in_insert && !self.dot.replaying {
            self.dot.rec.push(key.to_owned());
        }
        let e = &mut self.edit;
        match key {
            "<esc>" | "<c-[>" | "<c-c>" => {
                self.commit_edit();
                self.mode = Mode::Normal;
                if self.dot.in_insert && !self.dot.replaying {
                    self.dot.last = std::mem::take(&mut self.dot.rec);
                }
                self.dot.in_insert = false;
                if !self.dot.replaying {
                    self.commit();
                }
            }
            "<cr>" | "<down>" => {
                self.commit_edit();
                if self.row == self.last_row() && key == "<cr>" {
                    self.insert_rows(self.row + 1, vec![vec![String::new()]]);
                }
                self.row = (self.row + 1).min(self.last_row());
                self.begin_insert(true);
            }
            "<up>" => {
                self.commit_edit();
                self.row = self.row.saturating_sub(1);
                self.begin_insert(true);
            }
            "<tab>" => {
                self.commit_edit();
                if self.col < self.last_col() {
                    self.col += 1;
                } else if self.row < self.last_row() {
                    self.row += 1;
                    self.col = 0;
                }
                self.begin_insert(true);
            }
            "<s-tab>" => {
                self.commit_edit();
                self.col = self.col.saturating_sub(1);
                self.begin_insert(true);
            }
            "<bs>" | "<c-h>" => e.backspace(),
            "<del>" => e.delete(),
            "<left>" => e.left(),
            "<right>" => e.right(),
            "<home>" | "<c-a>" => e.home(),
            "<end>" | "<c-e>" => e.end(),
            "<c-w>" => e.delete_word_before(),
            "<c-u>" => e.delete_to_start(),
            "<c-k>" => e.delete_to_end(),
            "<c-left>" | "<c-b>" | "<a-left>" => e.word_left(),
            "<c-right>" | "<c-f>" | "<a-right>" => e.word_right(),
            "<a-bs>" => e.delete_word_before(),
            "<d-backspace>" => e.delete_to_start(),
            "<d-v>" | "<c-v>" => {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    let line = text.trim_end_matches(['\n', '\r']).replace(['\t', '\n', '\r'], " ");
                    self.edit.insert(&line);
                }
            }
            k if is_text(k) => e.insert(k),
            _ => {}
        }
    }

    // ---- command line ------------------------------------------------------

    fn cmdline_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let e = &mut self.cmdline;
        match key {
            "<esc>" | "<c-[>" | "<c-c>" => self.mode = Mode::Normal,
            "<bs>" | "<c-h>" if e.text.is_empty() => self.mode = Mode::Normal,
            "<cr>" => {
                let text = std::mem::take(&mut e.text);
                let mode = self.mode;
                self.mode = Mode::Normal;
                if !text.is_empty() && self.cmd_history.last() != Some(&text) {
                    self.cmd_history.push(text.clone());
                }
                match mode {
                    Mode::Search { forward } => self.start_search(&text, forward),
                    _ => self.ex(&text, cx),
                }
                self.commit();
                self.refilter();
            }
            "<up>" | "<down>" | "<c-p>" | "<c-n>" => {
                let len = self.cmd_history.len();
                if len == 0 {
                    return;
                }
                let up = matches!(key, "<up>" | "<c-p>");
                let pos = match (self.cmd_history_pos, up) {
                    (None, true) => Some(len - 1),
                    (None, false) => None,
                    (Some(p), true) => Some(p.saturating_sub(1)),
                    (Some(p), false) => (p + 1 < len).then_some(p + 1),
                };
                self.cmd_history_pos = pos;
                self.cmdline = LineEdit::new(pos.map_or("", |p| &self.cmd_history[p]), true);
            }
            "<bs>" | "<c-h>" => e.backspace(),
            "<del>" => e.delete(),
            "<left>" => e.left(),
            "<right>" => e.right(),
            "<home>" | "<c-b>" => e.home(),
            "<end>" | "<c-e>" => e.end(),
            "<c-w>" => e.delete_word_before(),
            "<c-u>" => e.delete_to_start(),
            "<d-v>" | "<c-v>" => {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    self.cmdline.insert(text.lines().next().unwrap_or(""));
                }
            }
            k if is_text(k) => e.insert(k),
            _ => {}
        }
    }

    // ---- fuzzy filter --------------------------------------------------------

    fn filter_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let e = &mut self.cmdline;
        match key {
            "<esc>" | "<c-[>" | "<c-c>" => {
                self.set_filter(String::new());
                self.mode = Mode::Normal;
                return;
            }
            "<cr>" => {
                self.mode = Mode::Normal;
                return;
            }
            "<bs>" | "<c-h>" if e.text.is_empty() => {
                self.mode = Mode::Normal;
                return;
            }
            "<tab>" | "<down>" | "<c-n>" => return self.filter_next(true),
            "<s-tab>" | "<up>" | "<c-p>" => return self.filter_next(false),
            "<bs>" | "<c-h>" => e.backspace(),
            "<del>" => e.delete(),
            "<left>" => e.left(),
            "<right>" => e.right(),
            "<home>" | "<c-b>" => e.home(),
            "<end>" | "<c-e>" => e.end(),
            "<c-w>" => e.delete_word_before(),
            "<c-u>" => e.delete_to_start(),
            "<d-v>" | "<c-v>" => {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    e.insert(text.lines().next().unwrap_or(""));
                }
            }
            k if is_text(k) => e.insert(k),
            _ => return,
        }
        self.set_filter(self.cmdline.text.clone());
        // Like incsearch: keep the cursor on a matching cell while typing.
        if !self.filter.trim().is_empty() && !fuzzy_match(&self.filter, self.cell()) {
            self.filter_next(true);
        }
    }

    fn filter_next(&mut self, forward: bool) {
        let q = self.filter.clone();
        match self.find(|s| !s.is_empty() && fuzzy_match(&q, s), forward) {
            Some((r, c, _)) => (self.row, self.col) = (r, c),
            None => self.set_err(format!("No match: {q}")),
        }
    }

    // ---- search --------------------------------------------------------------

    fn start_search(&mut self, text: &str, forward: bool) {
        let pattern = if text.is_empty() {
            match &self.search {
                Some(s) => s.pattern.clone(),
                None => return self.set_err("E35: No previous regular expression"),
            }
        } else {
            text.to_owned()
        };
        self.search = Some(Search {
            re: compile_search(&pattern),
            pattern,
            forward,
        });
        self.highlight = true;
        self.search_next(false);
    }

    fn search_next(&mut self, reverse: bool) {
        let Some(s) = &self.search else {
            return self.set_err("E35: No previous regular expression");
        };
        let forward = s.forward != reverse;
        let label = format!("{}{}", if forward { '/' } else { '?' }, s.pattern);
        let re = &s.re;
        match self.find(|c| re.is_match(c), forward) {
            Some((r, c, wrapped)) => {
                (self.row, self.col) = (r, c);
                if wrapped {
                    self.set_err(format!("search hit {}, continuing", if forward { "BOTTOM" } else { "TOP" }));
                } else {
                    self.set_msg(label);
                }
            }
            None => self.set_err(format!("E486: Pattern not found: {}", &label[1..])),
        }
    }

    /// Find the next matching cell from the cursor: (row, col, wrapped).
    fn find(&self, is_match: impl Fn(&str) -> bool, forward: bool) -> Option<(usize, usize, bool)> {
        let t = &self.table;
        let nrows = t.nrows();
        let (r0, c0) = (self.row, self.col);
        for step in 0..=nrows {
            let r = if forward {
                (r0 + step) % nrows
            } else {
                (r0 + nrows - step % nrows) % nrows
            };
            if !is_match(&t.line(r)) {
                continue;
            }
            let cells: Vec<&str> = t.fields(r).collect();
            let ok = |c: usize| match (step, forward) {
                (0, true) => c > c0,
                (0, false) => c < c0,
                (s, true) if s == nrows => c <= c0,
                (s, false) if s == nrows => c >= c0,
                _ => true,
            };
            let hit = if forward {
                (0..cells.len()).find(|&c| ok(c) && is_match(cells[c]))
            } else {
                (0..cells.len()).rev().find(|&c| ok(c) && is_match(cells[c]))
            };
            if let Some(c) = hit {
                let wrapped = if forward { r < r0 || (r == r0 && step > 0) } else { r > r0 || (r == r0 && step > 0) };
                return Some((r, c, wrapped));
            }
        }
        None
    }

    // ---- ex commands -------------------------------------------------------

    fn file_info(&mut self) {
        let name = self
            .path
            .as_ref()
            .map_or("[No Name]".into(), |p| p.display().to_string());
        self.set_msg(format!(
            "\"{name}\"{} {} rows × {} cols --{}%--",
            if self.is_dirty() { " [Modified]" } else { "" },
            self.table.nrows(),
            self.table.ncols(),
            (self.row + 1) * 100 / self.table.nrows()
        ));
    }

    /// Parse a leading line range: "%", "N", "N,M", ".", "$", ".+N".
    fn parse_range<'a>(&self, cmd: &'a str) -> (Option<(usize, usize)>, &'a str) {
        if let Some(rest) = cmd.strip_prefix('%') {
            return (Some((0, self.last_row())), rest);
        }
        let addr = |s: &str| -> Option<usize> {
            let (base, off) = match s.find(['+', '-']) {
                Some(i) if i > 0 => (&s[..i], &s[i..]),
                _ => (s, ""),
            };
            let base = match base {
                "." => self.row,
                "$" => self.last_row(),
                b => b.parse::<usize>().ok()?.saturating_sub(1),
            };
            let off: isize = if off.is_empty() { 0 } else { off.parse().ok()? };
            Some((base as isize + off).clamp(0, self.last_row() as isize) as usize)
        };
        let end = cmd
            .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '$' | '+' | '-' | ',')))
            .unwrap_or(cmd.len());
        let (spec, rest) = cmd.split_at(end);
        if spec.is_empty() {
            return (None, rest);
        }
        let range = match spec.split_once(',') {
            Some((a, b)) => addr(a).zip(addr(b)),
            None => addr(spec).map(|a| (a, a)),
        };
        (range.map(|(a, b)| (a.min(b), a.max(b))), rest)
    }

    pub fn ex(&mut self, input: &str, cx: &mut Context<Self>) {
        let input = input.trim();
        let (range, cmd) = self.parse_range(input);
        let cmd = cmd.trim();
        let (name, arg) = match cmd.find(|c: char| c.is_whitespace() || c == '/') {
            Some(i) => (&cmd[..i], cmd[i..].trim()),
            None => (cmd, ""),
        };
        let (name, bang) = match name.strip_suffix('!') {
            Some(n) => (n, true),
            None => (name, false),
        };
        match name {
            "" => {
                if let Some((_, r)) = range {
                    self.row = r;
                }
            }
            "w" | "write" | "wq" | "x" | "xit" | "exit" | "up" | "update" => {
                let quit = matches!(name, "wq" | "x" | "xit" | "exit");
                let skip = matches!(name, "x" | "xit" | "exit" | "up" | "update") && !self.is_dirty();
                if !skip && !self.write(arg) {
                    return;
                }
                if quit {
                    cx.quit();
                }
            }
            "q" | "quit" | "qa" | "qall" | "quita" | "quitall" | "cq" => {
                if self.is_dirty() && !bang && name != "cq" {
                    self.set_err("E37: No write since last change (add ! to override)");
                } else {
                    cx.quit();
                }
            }
            "e" | "edit" => self.edit_file(arg, bang),
            "d" | "delete" => {
                let (a, b) = range.unwrap_or((self.row, self.row));
                self.delete_rows(a, b);
            }
            "y" | "yank" => {
                let (a, b) = range.unwrap_or((self.row, self.row));
                self.yank_rows(a, b);
                self.set_msg(format!("{} rows yanked", b - a + 1));
            }
            "s" | "substitute" => self.substitute(range, arg),
            "sort" | "sor" => self.sort(range, bang, arg),
            "noh" | "nohlsearch" => self.highlight = false,
            "set" | "se" => self.set_option(arg),
            "header" => self.header = !self.header,
            "ic" | "ac" => {
                let at = if name == "ic" { self.col } else { self.col + 1 };
                let n: usize = arg.parse().unwrap_or(1);
                for _ in 0..n {
                    self.insert_col(at);
                }
                self.col = at;
            }
            "dc" => {
                let n: usize = arg.parse().unwrap_or(1);
                self.delete_cols(self.col, n);
            }
            "fit" => self.fit_all(),
            "width" | "wi" => match arg.parse::<i32>() {
                Ok(w) => {
                    self.resize_col(self.col, 0);
                    self.widths[self.col] = w.clamp(1, 500) as u16;
                }
                Err(_) => self.set_err("usage: :width N"),
            },
            "u" | "undo" => self.undo(false),
            "red" | "redo" => self.undo(true),
            "f" | "file" => self.file_info(),
            "h" | "help" => self.set_msg(HELP),
            _ => {
                if let Some((_, r)) = range.filter(|_| cmd.is_empty()) {
                    self.row = r;
                } else {
                    self.set_err(format!("E492: Not an editor command: {input}"));
                }
            }
        }
    }

    fn write(&mut self, arg: &str) -> bool {
        let path = if arg.is_empty() {
            match &self.path {
                Some(p) => p.clone(),
                None => {
                    self.set_err("E32: No file name");
                    return false;
                }
            }
        } else {
            PathBuf::from(expand_tilde(arg))
        };
        let start = std::time::Instant::now();
        let is_current = self.path.is_none() || self.path.as_ref() == Some(&path);
        match self.table.save(&path) {
            Ok(bytes) => {
                self.commit();
                if is_current {
                    self.history.saved_depth = self.history.undo.len();
                    self.path = Some(path.clone());
                }
                self.set_msg(format!(
                    "\"{}\" {} rows, {} bytes written in {:.0?}",
                    path.display(),
                    self.table.nrows(),
                    bytes,
                    start.elapsed()
                ));
                true
            }
            Err(e) => {
                self.set_err(format!("E212: Can't open file for writing: {e}"));
                false
            }
        }
    }

    fn edit_file(&mut self, arg: &str, bang: bool) {
        if self.is_dirty() && !bang {
            return self.set_err("E37: No write since last change (add ! to override)");
        }
        let path = if arg.is_empty() {
            match &self.path {
                Some(p) => p.clone(),
                None => return self.set_err("E32: No file name"),
            }
        } else {
            PathBuf::from(expand_tilde(arg))
        };
        if !path.exists() {
            self.replace_table(Table::default(), Some(path));
            return self.set_msg("[New]");
        }
        match Table::load(&path) {
            Ok((table, stats)) => {
                let msg = load_message(&path, &table, &stats);
                self.replace_table(table, Some(path));
                self.set_msg(msg);
            }
            Err(e) => self.set_err(format!("E484: Can't open file: {e}")),
        }
    }

    fn set_option(&mut self, arg: &str) {
        for opt in arg.split_whitespace() {
            match opt {
                "header" => self.header = true,
                "noheader" => self.header = false,
                "header!" | "invheader" => self.header = !self.header,
                "rnu" | "relativenumber" => self.relative = true,
                "nornu" | "norelativenumber" => self.relative = false,
                "rnu!" | "relativenumber!" => self.relative = !self.relative,
                "hls" | "hlsearch" => self.highlight = true,
                "nohls" | "nohlsearch" => self.highlight = false,
                _ => return self.set_err(format!("E518: Unknown option: {opt}")),
            }
        }
    }

    fn substitute(&mut self, range: Option<(usize, usize)>, arg: &str) {
        let Some(sep) = arg.chars().next() else {
            return self.set_err("usage: :s/pattern/replacement/[gi]");
        };
        let parts = split_unescaped(&arg[sep.len_utf8()..], sep);
        let pattern = parts.first().map(String::as_str).unwrap_or("");
        let pattern = if pattern.is_empty() {
            match &self.search {
                Some(s) => s.pattern.clone(),
                None => return self.set_err("E35: No previous regular expression"),
            }
        } else {
            pattern.to_owned()
        };
        let rep = vim_replacement(parts.get(1).map(String::as_str).unwrap_or(""));
        let flags = parts.get(2).map(String::as_str).unwrap_or("");
        let global = flags.contains('g');
        let re = if flags.contains('i') {
            Regex::new(&format!("(?i){pattern}"))
        } else if flags.contains('I') {
            Regex::new(&pattern)
        } else {
            Ok(compile_search(&pattern))
        };
        let re = match re {
            Ok(r) => r,
            Err(e) => return self.set_err(format!("E486: bad pattern: {e}")),
        };
        let (r0, r1) = range.unwrap_or((self.row, self.row));
        let (mut subs, mut rows) = (0usize, 0usize);
        for r in self.rows_in(r0, r1) {
            if !re.is_match(&self.table.line(r)) {
                continue;
            }
            let mut changed = Vec::new();
            for (c, cell) in self.table.fields(r).enumerate() {
                let n = if global {
                    re.find_iter(cell).count()
                } else {
                    re.is_match(cell) as usize
                };
                if n > 0 {
                    let new = if global {
                        re.replace_all(cell, rep.as_str())
                    } else {
                        re.replace(cell, rep.as_str())
                    };
                    if new != cell {
                        changed.push((c, new.into_owned()));
                    }
                    subs += n;
                }
            }
            if !changed.is_empty() {
                rows += 1;
                self.row = r;
            }
            for (c, v) in changed {
                self.set_cell(r, c, v);
            }
        }
        if subs == 0 {
            self.set_err(format!("E486: Pattern not found: {pattern}"));
        } else {
            self.set_msg(format!("{subs} substitutions on {rows} rows"));
        }
        self.search = Some(Search {
            re,
            pattern,
            forward: true,
        });
    }

    /// :sort[!] [n|r] — stable sort of rows by the cursor's column.
    /// Numbers sort numerically (before text) unless `r` (raw text) is given.
    fn sort(&mut self, range: Option<(usize, usize)>, reverse: bool, arg: &str) {
        let (r0, r1) = range.unwrap_or((self.first_data_row(), self.last_row()));
        if r1 <= r0 {
            return;
        }
        let col = self.col;
        let numeric = !arg.contains('r');
        let start = std::time::Instant::now();
        let mut keys: Vec<(Option<f64>, &str, usize)> = (r0..=r1)
            .map(|r| {
                let s = self.table.fields(r).nth(col).unwrap_or("");
                let num = if numeric { s.trim().parse::<f64>().ok().filter(|f| !f.is_nan()) } else { None };
                (num, s, r)
            })
            .collect();
        keys.sort_by(|a, b| {
            let ord = match (a.0, b.0) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.1.cmp(b.1),
            };
            if reverse { ord.reverse() } else { ord }
        });
        let order: Vec<usize> = keys.into_iter().map(|k| k.2).collect();
        if order.iter().enumerate().all(|(i, &r)| r == r0 + i) {
            return self.set_msg("already sorted");
        }
        let rows = order.iter().map(|&r| self.table.row(r).clone()).collect();
        self.record(Change::Splice {
            at: r0,
            remove: r1 - r0 + 1,
            insert: rows,
        });
        self.set_msg(format!(
            "sorted {} rows by {} in {:.0?}",
            r1 - r0 + 1,
            crate::editor::col_name(col),
            start.elapsed()
        ));
    }
}

const HELP: &str = "hjkl move · w/b next/prev filled cell · gg/G · {/} jump blocks · i/a/cc edit · o/O new row · dd/yy/p rows · x cell · tt/xx add/delete column · dc/yc column · v/V select · u/^r or ⌘Z/⌘⇧Z undo/redo · ⌘+/⌘− add/delete column · ⌥←/⌥→ add column · ⌥↑/⌥↓ add row · ⌘⇧⌫ delete column · ⌘⌫ delete row · click A/B/… or 1/2/… to select · / search · Space Space fuzzy filter · :w :q :sort :%s/a/b/g :ic :ac :dc :set header!";

pub fn load_message(path: &std::path::Path, table: &Table, stats: &crate::table::LoadStats) -> String {
    format!(
        "\"{}\" {} rows × {} cols, {:.1} MB loaded in {:.0?}",
        path.display(),
        table.nrows(),
        table.ncols(),
        stats.bytes as f64 / 1e6,
        stats.elapsed
    )
}

fn expand_tilde(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => p.to_owned(),
    }
}

fn split_unescaped(s: &str, sep: char) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some(n) if n == sep => parts.last_mut().unwrap().push(n),
                Some(n) => {
                    let p = parts.last_mut().unwrap();
                    p.push('\\');
                    p.push(n);
                }
                None => parts.last_mut().unwrap().push('\\'),
            }
        } else if ch == sep {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(ch);
        }
    }
    parts
}

/// Convert vim replacement syntax (`&`, `\1`) into the regex crate's (`$0`, `${1}`).
fn vim_replacement(rep: &str) -> String {
    let mut out = String::new();
    let mut chars = rep.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '$' => out.push_str("$$"),
            '&' => out.push_str("${0}"),
            '\\' => match chars.next() {
                Some(d) if d.is_ascii_digit() => out.push_str(&format!("${{{d}}}")),
                Some('t') => out.push('\t'),
                Some(c) => out.push(c),
                None => out.push('\\'),
            },
            c => out.push(c),
        }
    }
    out
}

fn toggle_case(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            let v: Vec<char> = if c.is_uppercase() {
                c.to_lowercase().collect()
            } else {
                c.to_uppercase().collect()
            };
            v
        })
        .collect()
}

/// vim's ctrl-a: add `delta` to the first integer in the cell.
fn increment(s: &str, delta: i64) -> Option<String> {
    let b = s.as_bytes();
    let start = b.iter().position(u8::is_ascii_digit)?;
    let end = start + b[start..].iter().take_while(|c| c.is_ascii_digit()).count();
    let neg = start > 0 && b[start - 1] == b'-';
    let num_start = if neg { start - 1 } else { start };
    let n: i64 = s[num_start..end].parse().ok()?;
    let digits = end - start;
    let v = n.saturating_add(delta);
    // Keep zero padding ("007" -> "008").
    let formatted = if b[start] == b'0' && digits > 1 {
        let sign = if v < 0 { "-" } else { "" };
        format!("{sign}{:0digits$}", v.unsigned_abs())
    } else {
        v.to_string()
    };
    Some(format!("{}{}{}", &s[..num_start], formatted, &s[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts() {
        assert_eq!(split_count(&["1", "2", "j"]), (Some(12), &["j"][..]));
        assert_eq!(split_count(&["0"]), (None, &["0"][..]));
    }

    #[test]
    fn incr() {
        assert_eq!(increment("item 9", 1).unwrap(), "item 10");
        assert_eq!(increment("-3", 5).unwrap(), "2");
        assert_eq!(increment("007", 1).unwrap(), "008");
        assert!(increment("abc", 1).is_none());
    }

    #[test]
    fn fuzzy() {
        assert!(fuzzy_match("ali", "Alice"));
        assert!(fuzzy_match("ace", "Alice"));
        assert!(!fuzzy_match("eca", "Alice"));
        assert!(!fuzzy_match("Ali", "alice"), "smartcase");
        assert!(fuzzy_match("al ce", "Alice"), "terms are ANDed");
        assert!(!fuzzy_match("al x", "Alice"));
        assert!(fuzzy_match("  ", "anything"));
    }

    #[test]
    fn replacement() {
        assert_eq!(vim_replacement(r"<\1>&$"), "<${1}>${0}$$");
        assert_eq!(split_unescaped(r"a\/b/c/g", '/'), vec!["a/b", "c", "g"]);
    }
}
