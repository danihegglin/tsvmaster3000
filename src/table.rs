//! Table storage.
//!
//! The file is kept as one contiguous `String`; every row starts out as a
//! `Row::Raw` byte range into it, so loading is a single SIMD scan for tabs
//! and newlines and nothing is split or allocated per cell. A row is only
//! materialized into `Row::Owned` when it is edited.

use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub enum Row {
    Raw { start: usize, end: usize },
    Owned(Vec<String>),
}

/// A reversible edit. `Table::apply` performs it and returns its inverse.
#[derive(Debug)]
pub enum Change {
    Cell {
        row: usize,
        col: usize,
        value: String,
        /// Truncate the row to this many cells afterwards (undoes padding).
        shrink_to: Option<usize>,
    },
    Splice {
        at: usize,
        remove: usize,
        insert: Vec<Row>,
    },
    /// Insert a column. `values: None` inserts "" into every row long enough
    /// to have a cell at `at`; `Some` restores the exact per-row values.
    InsertCol {
        at: usize,
        values: Option<Vec<Option<String>>>,
    },
    RemoveCol {
        at: usize,
    },
}

pub struct LoadStats {
    pub bytes: usize,
    pub elapsed: Duration,
}

pub struct Table {
    buf: String,
    rows: Vec<Row>,
    ncols: usize,
    crlf: bool,
    trailing_newline: bool,
}

impl Default for Table {
    fn default() -> Self {
        Self {
            buf: String::new(),
            rows: vec![Row::Owned(vec![String::new()])],
            ncols: 1,
            crlf: false,
            trailing_newline: true,
        }
    }
}

impl Table {
    pub fn load(path: &Path) -> io::Result<(Table, LoadStats)> {
        let start = Instant::now();
        let bytes = fs::read(path)?;
        let len = bytes.len();
        let table = Table::from_bytes(bytes);
        Ok((
            table,
            LoadStats {
                bytes: len,
                elapsed: start.elapsed(),
            },
        ))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Table {
        let buf = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        };
        if buf.is_empty() {
            return Table::default();
        }
        let b = buf.as_bytes();
        let mut rows = Vec::with_capacity(b.len() / 48 + 1);
        let (mut start, mut tabs, mut max_tabs, mut crs) = (0, 0, 0, 0);
        for i in memchr::memchr2_iter(b'\t', b'\n', b) {
            if b[i] == b'\t' {
                tabs += 1;
                continue;
            }
            let mut end = i;
            if end > start && b[end - 1] == b'\r' {
                end -= 1;
                crs += 1;
            }
            rows.push(Row::Raw { start, end });
            max_tabs = max_tabs.max(tabs);
            tabs = 0;
            start = i + 1;
        }
        let trailing_newline = start == b.len();
        if !trailing_newline {
            rows.push(Row::Raw { start, end: b.len() });
            max_tabs = max_tabs.max(tabs);
        }
        let crlf = crs > 0 && crs * 2 >= rows.len();
        Table {
            buf,
            rows,
            ncols: max_tabs + 1,
            crlf,
            trailing_newline,
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<usize> {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = path.with_file_name(format!(".{file_name}.tsv-tmp"));
        let written = {
            let file = fs::File::create(&tmp)?;
            let mut w = BufWriter::with_capacity(1 << 20, file);
            let n = self.write_to(&mut w)?;
            w.into_inner().map_err(|e| e.into_error())?.sync_all()?;
            n
        };
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        fs::rename(&tmp, path)?;
        Ok(written)
    }

    fn write_to(&self, w: &mut impl Write) -> io::Result<usize> {
        let nl: &[u8] = if self.crlf { b"\r\n" } else { b"\n" };
        let buf = self.buf.as_bytes();
        let mut total = 0;
        // Runs of untouched, contiguous raw rows are copied in one write.
        let mut run: Option<(usize, usize)> = None;
        let last = self.rows.len().saturating_sub(1);
        for (i, row) in self.rows.iter().enumerate() {
            let newline = i < last || self.trailing_newline;
            match row {
                Row::Raw { start, end } => {
                    // Include the original line terminator when it matches ours.
                    let term_end = if newline && buf.get(*end..end + nl.len()) == Some(nl) {
                        end + nl.len()
                    } else {
                        *end
                    };
                    match &mut run {
                        Some((_, e)) if *e == *start => *e = term_end,
                        _ => {
                            if let Some((s, e)) = run.take() {
                                w.write_all(&buf[s..e])?;
                                total += e - s;
                            }
                            run = Some((*start, term_end));
                        }
                    }
                    if term_end == *end && newline {
                        let (s, e) = run.take().unwrap();
                        w.write_all(&buf[s..e])?;
                        w.write_all(nl)?;
                        total += e - s + nl.len();
                    }
                }
                Row::Owned(cells) => {
                    if let Some((s, e)) = run.take() {
                        w.write_all(&buf[s..e])?;
                        total += e - s;
                    }
                    for (j, cell) in cells.iter().enumerate() {
                        if j > 0 {
                            w.write_all(b"\t")?;
                        }
                        w.write_all(cell.as_bytes())?;
                        total += cell.len() + (j > 0) as usize;
                    }
                    if newline {
                        w.write_all(nl)?;
                        total += nl.len();
                    }
                }
            }
        }
        if let Some((s, e)) = run {
            w.write_all(&buf[s..e])?;
            total += e - s;
        }
        Ok(total)
    }

    pub fn nrows(&self) -> usize {
        self.rows.len()
    }

    pub fn ncols(&self) -> usize {
        self.ncols
    }

    pub fn row(&self, r: usize) -> &Row {
        &self.rows[r]
    }

    /// The row as one line of text (only borrowed for raw rows).
    pub fn line(&self, r: usize) -> std::borrow::Cow<'_, str> {
        match &self.rows[r] {
            Row::Raw { start, end } => std::borrow::Cow::Borrowed(&self.buf[*start..*end]),
            Row::Owned(cells) => std::borrow::Cow::Owned(cells.join("\t")),
        }
    }

    pub fn fields(&self, r: usize) -> Fields<'_> {
        match &self.rows[r] {
            Row::Raw { start, end } => Fields::Raw(self.buf[*start..*end].split('\t')),
            Row::Owned(cells) => Fields::Owned(cells.iter()),
        }
    }

    pub fn row_len(&self, r: usize) -> usize {
        match &self.rows[r] {
            Row::Raw { start, end } => {
                memchr::memchr_iter(b'\t', &self.buf.as_bytes()[*start..*end]).count() + 1
            }
            Row::Owned(cells) => cells.len(),
        }
    }

    pub fn cell(&self, r: usize, c: usize) -> &str {
        match &self.rows[r] {
            Row::Raw { .. } => self.fields(r).nth(c).unwrap_or(""),
            Row::Owned(cells) => cells.get(c).map(|s| s.as_str()).unwrap_or(""),
        }
    }

    pub fn row_cells(&self, r: usize) -> Vec<String> {
        self.fields(r).map(str::to_owned).collect()
    }

    fn materialize(&mut self, r: usize) -> &mut Vec<String> {
        if let Row::Raw { start, end } = self.rows[r] {
            let cells = self.buf[start..end].split('\t').map(str::to_owned).collect();
            self.rows[r] = Row::Owned(cells);
        }
        match &mut self.rows[r] {
            Row::Owned(cells) => cells,
            Row::Raw { .. } => unreachable!(),
        }
    }

    fn cells_in(&self, row: &Row) -> usize {
        match row {
            Row::Raw { start, end } => {
                memchr::memchr_iter(b'\t', &self.buf.as_bytes()[*start..*end]).count() + 1
            }
            Row::Owned(cells) => cells.len(),
        }
    }

    pub fn apply(&mut self, change: Change) -> Change {
        match change {
            Change::Cell {
                row,
                col,
                value,
                shrink_to,
            } => {
                let cells = self.materialize(row);
                let old_len = cells.len();
                if cells.len() <= col {
                    cells.resize(col + 1, String::new());
                }
                let old = std::mem::replace(&mut cells[col], value);
                if let Some(n) = shrink_to {
                    cells.truncate(n.max(1));
                }
                self.ncols = self.ncols.max(col + 1);
                Change::Cell {
                    row,
                    col,
                    value: old,
                    shrink_to: (old_len <= col).then_some(old_len),
                }
            }
            Change::Splice { at, remove, insert } => {
                for row in &insert {
                    self.ncols = self.ncols.max(self.cells_in(row));
                }
                let n = insert.len();
                let removed: Vec<Row> = self.rows.splice(at..at + remove, insert).collect();
                if self.rows.is_empty() {
                    // A table always has at least one row.
                    self.rows.push(Row::Owned(vec![String::new()]));
                    return Change::Splice {
                        at: 0,
                        remove: 1,
                        insert: removed,
                    };
                }
                Change::Splice {
                    at,
                    remove: n,
                    insert: removed,
                }
            }
            Change::InsertCol { at, values } => {
                for r in 0..self.rows.len() {
                    let value = match &values {
                        Some(v) => v[r].clone(),
                        None => (self.cells_in(&self.rows[r]) >= at).then(String::new),
                    };
                    if let Some(value) = value {
                        let cells = self.materialize(r);
                        if cells.len() < at {
                            cells.resize(at, String::new());
                        }
                        cells.insert(at, value);
                    }
                }
                self.ncols += 1;
                Change::RemoveCol { at }
            }
            Change::RemoveCol { at } => {
                let mut values = Vec::with_capacity(self.rows.len());
                for r in 0..self.rows.len() {
                    if self.cells_in(&self.rows[r]) > at {
                        let cells = self.materialize(r);
                        values.push(Some(cells.remove(at)));
                    } else {
                        values.push(None);
                    }
                }
                self.ncols = self.ncols.saturating_sub(1).max(1);
                Change::InsertCol {
                    at,
                    values: Some(values),
                }
            }
        }
    }
}

pub enum Fields<'a> {
    Raw(std::str::Split<'a, char>),
    Owned(std::slice::Iter<'a, String>),
}

impl<'a> Iterator for Fields<'a> {
    type Item = &'a str;

    #[inline]
    fn next(&mut self) -> Option<&'a str> {
        match self {
            Fields::Raw(split) => split.next(),
            Fields::Owned(iter) => iter.next().map(String::as_str),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(input: &str) -> String {
        let t = Table::from_bytes(input.as_bytes().to_vec());
        let mut out = Vec::new();
        t.write_to(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn roundtrips_untouched() {
        for s in ["a\tb\nc\td\n", "a\tb\nc", "x\r\ny\r\n", "\n\n", "a\t\t\n"] {
            assert_eq!(roundtrip(s), s);
        }
    }

    #[test]
    fn edit_and_undo() {
        let mut t = Table::from_bytes(b"a\tb\nc\n".to_vec());
        assert_eq!(t.ncols(), 2);
        let inv = t.apply(Change::Cell {
            row: 1,
            col: 2,
            value: "z".into(),
            shrink_to: None,
        });
        assert_eq!(t.cell(1, 2), "z");
        assert_eq!(t.ncols(), 3);
        t.apply(inv);
        let mut out = Vec::new();
        t.write_to(&mut out).unwrap();
        assert_eq!(out, b"a\tb\nc\n");
    }

    #[test]
    fn columns() {
        let mut t = Table::from_bytes(b"a\tb\nc\n".to_vec());
        let inv = t.apply(Change::InsertCol { at: 1, values: None });
        assert_eq!(t.row_cells(0), ["a", "", "b"]);
        assert_eq!(t.row_cells(1), ["c", ""]);
        t.apply(inv);
        assert_eq!(t.row_cells(0), ["a", "b"]);
        assert_eq!(t.row_cells(1), ["c"]);
        let inv = t.apply(Change::RemoveCol { at: 0 });
        assert_eq!(t.row_cells(0), ["b"]);
        t.apply(inv);
        assert_eq!(t.row_cells(0), ["a", "b"]);
    }

    #[test]
    fn splice() {
        let mut t = Table::from_bytes(b"1\n2\n3\n".to_vec());
        let inv = t.apply(Change::Splice {
            at: 0,
            remove: 3,
            insert: vec![],
        });
        assert_eq!(t.nrows(), 1);
        t.apply(inv);
        assert_eq!(t.nrows(), 3);
        assert_eq!(t.cell(2, 0), "3");
    }
}

#[cfg(test)]
mod bench {
    /// `TSV_BENCH=file.tsv cargo test --release bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn load_save() {
        let path = std::path::PathBuf::from(std::env::var("TSV_BENCH").unwrap());
        let (t, stats) = super::Table::load(&path).unwrap();
        println!("load: {} rows in {:?}", t.nrows(), stats.elapsed);
        let out = path.with_extension("out.tsv");
        let start = std::time::Instant::now();
        t.save(&out).unwrap();
        println!("save: {:?}", start.elapsed());
        assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&out).unwrap());
        std::fs::remove_file(out).unwrap();
    }
}
