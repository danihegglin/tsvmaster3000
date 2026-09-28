//! Editor-level tests: drive the editor with key tokens, as the key handler
//! would, and check the table.

use gpui::{AppContext, Entity, TestAppContext};

use crate::editor::Editor;
use crate::table::Table;

const TSV: &str = "id\tname\tcity\n1\tcarol\tOslo\n2\tdave\tBerlin\n3\toscar\tToronto\n";

fn editor(cx: &mut TestAppContext) -> Entity<Editor> {
    let table = Table::from_bytes(TSV.as_bytes().to_vec());
    cx.update(|cx| cx.new(|cx| Editor::new(table, None, None, cx)))
}

fn keys(e: &Entity<Editor>, cx: &mut TestAppContext, keys: &[&str]) {
    e.update(cx, |e, cx| {
        for k in keys {
            e.handle_key(k, cx);
            e.flush_register(cx);
        }
    });
}

/// The table as "a,b,c|d,e,f" (rows padded to the column count).
fn dump(e: &Entity<Editor>, cx: &mut TestAppContext) -> String {
    e.read_with(cx, |e, _| {
        let t = &e.table;
        (0..t.nrows())
            .map(|r| (0..t.ncols()).map(|c| t.cell(r, c)).collect::<Vec<_>>().join(","))
            .collect::<Vec<_>>()
            .join("|")
    })
}

const START: &str = "id,name,city|1,carol,Oslo|2,dave,Berlin|3,oscar,Toronto";

/// Run `action` from the start state (cursor on B2), expect `after`, then
/// check that undo restores the start and redo restores `after`, with both
/// the vim keys and the ⌘ shortcuts.
fn roundtrip(cx: &mut TestAppContext, action: &[&str], after: &str) {
    for (undo, redo) in [("u", "<c-r>"), ("<d-z>", "<d-s-z>"), ("<d-z>", "<d-y>")] {
        let e = editor(cx);
        keys(&e, cx, &["j", "l"]);
        keys(&e, cx, action);
        assert_eq!(dump(&e, cx), after, "{action:?}");
        keys(&e, cx, &[undo]);
        assert_eq!(dump(&e, cx), START, "{action:?} then {undo}");
        keys(&e, cx, &[redo]);
        assert_eq!(dump(&e, cx), after, "{action:?} then {undo} {redo}");
        keys(&e, cx, &[undo]);
        assert_eq!(dump(&e, cx), START, "{action:?} then {undo} {redo} {undo}");
    }
}

#[gpui::test]
fn add_column(cx: &mut TestAppContext) {
    let after = "id,name,,city|1,carol,,Oslo|2,dave,,Berlin|3,oscar,,Toronto";
    roundtrip(cx, &["t", "t"], after);
    roundtrip(cx, &["<d-=>"], after);
    roundtrip(cx, &["<a-right>"], after);
    roundtrip(
        cx,
        &["<a-left>"],
        "id,,name,city|1,,carol,Oslo|2,,dave,Berlin|3,,oscar,Toronto",
    );
    roundtrip(cx, &[":", "a", "c", "<cr>"], after);
}

#[gpui::test]
fn delete_column(cx: &mut TestAppContext) {
    let after = "id,city|1,Oslo|2,Berlin|3,Toronto";
    roundtrip(cx, &["x", "x"], after);
    roundtrip(cx, &["<d-->"], after);
    roundtrip(cx, &["<d-s-backspace>"], after);
    roundtrip(cx, &["d", "c"], after);
    roundtrip(cx, &[":", "d", "c", "<cr>"], after);
    // Two columns from a visual selection.
    roundtrip(cx, &["v", "l", "<d-->"], "id|1|2|3");
}

#[gpui::test]
fn add_row(cx: &mut TestAppContext) {
    let below = "id,name,city|1,carol,Oslo|,,|2,dave,Berlin|3,oscar,Toronto";
    let above = "id,name,city|,,|1,carol,Oslo|2,dave,Berlin|3,oscar,Toronto";
    roundtrip(cx, &["o", "<esc>"], below);
    roundtrip(cx, &["O", "<esc>"], above);
    roundtrip(cx, &["<d-enter>"], below);
    roundtrip(cx, &["<d-s-enter>"], above);
    roundtrip(cx, &["<a-down>"], below);
    roundtrip(cx, &["<a-up>"], above);
}

#[gpui::test]
fn delete_row(cx: &mut TestAppContext) {
    let after = "id,name,city|2,dave,Berlin|3,oscar,Toronto";
    roundtrip(cx, &["d", "d"], after);
    roundtrip(cx, &["<d-backspace>"], after);
    roundtrip(cx, &[":", "d", "<cr>"], after);
    roundtrip(cx, &["V", "j", "<d-backspace>"], "id,name,city|3,oscar,Toronto");
}

/// A sequence of structural edits unwinds step by step, and redoes forward.
#[gpui::test]
fn mixed_history(cx: &mut TestAppContext) {
    let e = editor(cx);
    keys(&e, cx, &["j", "l"]);
    let mut states = vec![dump(&e, cx)];
    for action in [&["t", "t"][..], &["o", "<esc>"], &["x", "x"], &["d", "d"], &["<d-=>"]] {
        keys(&e, cx, action);
        states.push(dump(&e, cx));
    }
    for want in states.iter().rev().skip(1) {
        keys(&e, cx, &["u"]);
        assert_eq!(&dump(&e, cx), want);
    }
    for want in states.iter().skip(1) {
        keys(&e, cx, &["<c-r>"]);
        assert_eq!(&dump(&e, cx), want);
    }
}

#[gpui::test]
fn column_grows_while_typing(cx: &mut TestAppContext) {
    let e = editor(cx);
    keys(&e, cx, &["j", "l"]); // B2 "carol", column fit to 5
    let width = |e: &Entity<Editor>, cx: &mut TestAppContext| e.read_with(cx, |e, _| e.col_chars(1));
    assert_eq!(width(&e, cx), 5);
    keys(&e, cx, &["a"]);
    assert_eq!(width(&e, cx), 6, "room for the caret");
    let long = "a much longer name than the column had room for, over forty chars";
    let chars: Vec<String> = long.chars().map(String::from).collect();
    keys(&e, cx, &chars.iter().map(String::as_str).collect::<Vec<_>>());
    let typed = "carol".len() + long.len();
    assert_eq!(width(&e, cx), typed + 1);
    // Deleting text shrinks it back, but never below the column's width.
    keys(&e, cx, &["<c-u>"]);
    assert_eq!(width(&e, cx), 5);
    keys(&e, cx, &chars.iter().map(String::as_str).collect::<Vec<_>>());
    keys(&e, cx, &["<esc>"]);
    assert_eq!(width(&e, cx), long.len(), "committed width sticks");
}

#[gpui::test]
fn space_space_filters(cx: &mut TestAppContext) {
    use crate::editor::Mode;
    let e = editor(cx);
    let state = |e: &Entity<Editor>, cx: &mut TestAppContext| {
        e.read_with(cx, |e, _| (e.mode, e.filter.clone(), e.row, e.col))
    };
    keys(&e, cx, &[" "]);
    assert_eq!(state(&e, cx), (Mode::Normal, String::new(), 0, 0), "one space waits");
    keys(&e, cx, &[" ", "b", "l", "n"]);
    assert_eq!(state(&e, cx), (Mode::Filter, "bln".into(), 2, 2), "jumps to Berlin");
    keys(&e, cx, &["<cr>"]);
    assert_eq!(state(&e, cx), (Mode::Normal, "bln".into(), 2, 2), "Enter keeps the filter");
    keys(&e, cx, &["<esc>"]);
    assert_eq!(state(&e, cx), (Mode::Normal, String::new(), 2, 2), "Esc clears it");
}

fn shown(e: &Entity<Editor>, cx: &mut TestAppContext) -> Option<Vec<usize>> {
    e.read_with(cx, |e, _| e.shown.clone())
}

fn row(e: &Entity<Editor>, cx: &mut TestAppContext) -> usize {
    e.read_with(cx, |e, _| e.row)
}

/// Filter "os": matches Oslo and oscar, hides dave/Berlin (row 2).
fn filtered(cx: &mut TestAppContext) -> Entity<Editor> {
    let e = editor(cx);
    keys(&e, cx, &["j", " ", " ", "o", "s", "<cr>"]);
    assert_eq!(shown(&e, cx), Some(vec![0, 1, 3]));
    e
}

#[gpui::test]
fn filter_hides_rows(cx: &mut TestAppContext) {
    let e = filtered(cx);
    assert_eq!(row(&e, cx), 1);
    keys(&e, cx, &["j"]);
    assert_eq!(row(&e, cx), 3, "j skips the hidden row");
    keys(&e, cx, &["k"]);
    assert_eq!(row(&e, cx), 1);
    keys(&e, cx, &["G"]);
    assert_eq!(row(&e, cx), 3);
    // Narrowing further, then widening again.
    keys(&e, cx, &[" ", " ", "c"]);
    assert_eq!(shown(&e, cx), Some(vec![0, 3]), "only oscar");
    keys(&e, cx, &["<bs>", "<bs>", "<bs>"]);
    assert_eq!(shown(&e, cx), None, "empty query shows everything");
    keys(&e, cx, &[" ", " ", "z", "z", "z"]);
    assert_eq!(shown(&e, cx).map(|s| s.len()), Some(2), "no match keeps one blank row");
    keys(&e, cx, &["<esc>"]);
    assert_eq!(shown(&e, cx), None);
}

#[gpui::test]
fn filtered_edits_skip_hidden_rows(cx: &mut TestAppContext) {
    let e = filtered(cx);
    // Selecting "both" visible rows and deleting leaves the hidden one.
    keys(&e, cx, &["V", "j", "d"]);
    assert_eq!(dump(&e, cx), "id,name,city|2,dave,Berlin");
    keys(&e, cx, &["<esc>", "u"]);
    assert_eq!(dump(&e, cx), START);

    let e = filtered(cx);
    keys(&e, cx, &["d", "j"]);
    assert_eq!(dump(&e, cx), "id,name,city|2,dave,Berlin", "dj too");

    let e = filtered(cx);
    keys(&e, cx, &["V", "j", "~"]);
    assert_eq!(dump(&e, cx), "id,name,city|1,CAROL,oSLO|2,dave,Berlin|3,OSCAR,tORONTO");

    // Column deletes still take the whole column, and undo brings it back.
    let e = filtered(cx);
    keys(&e, cx, &["l", "l", "x", "x"]);
    assert_eq!(dump(&e, cx), "id,name|1,carol|2,dave|3,oscar");
    keys(&e, cx, &["u"]);
    assert_eq!(dump(&e, cx), START);
}

#[gpui::test]
fn filtered_rows_follow_inserts(cx: &mut TestAppContext) {
    let e = filtered(cx);
    keys(&e, cx, &["O", "<esc>"]);
    assert_eq!(shown(&e, cx), Some(vec![0, 1, 2, 4]), "new row shown, later rows shift");
    keys(&e, cx, &["u"]);
    assert_eq!(shown(&e, cx), Some(vec![0, 1, 3]));
    assert_eq!(dump(&e, cx), START);
}
