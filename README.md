<div align="center">

<pre>
████████╗ ███████╗ ██╗   ██╗   ███╗   ███╗  █████╗  ███████╗ ████████╗ ███████╗ ██████╗ 
╚══██╔══╝ ██╔════╝ ██║   ██║   ████╗ ████║ ██╔══██╗ ██╔════╝ ╚══██╔══╝ ██╔════╝ ██╔══██╗
   ██║    ███████╗ ██║   ██║   ██╔████╔██║ ███████║ ███████╗    ██║    █████╗   ██████╔╝
   ██║    ╚════██║ ╚██╗ ██╔╝   ██║╚██╔╝██║ ██╔══██║ ╚════██║    ██║    ██╔══╝   ██╔══██╗
   ██║    ███████║  ╚████╔╝    ██║ ╚═╝ ██║ ██║  ██║ ███████║    ██║    ███████╗ ██║  ██║
   ╚═╝    ╚══════╝   ╚═══╝     ╚═╝     ╚═╝ ╚═╝  ╚═╝ ╚══════╝    ╚═╝    ╚══════╝ ╚═╝  ╚═╝
                                                                                        
                                                  ██████╗   ██████╗   ██████╗   ██████╗ 
                                                  ╚════██╗ ██╔═████╗ ██╔═████╗ ██╔═████╗
                                                   █████╔╝ ██║██╔██║ ██║██╔██║ ██║██╔██║
                                                   ╚═══██╗ ████╔╝██║ ████╔╝██║ ████╔╝██║
                                                  ██████╔╝ ╚██████╔╝ ╚██████╔╝ ╚██████╔╝
                                                  ╚═════╝   ╚═════╝   ╚═════╝   ╚═════╝ 
</pre>

<b>▓▒░ a fast terminal TSV editor · vim-style keys · color-coded columns ░▒▓</b>

<br><br>

<img alt="rust: 2021 edition" src="https://img.shields.io/badge/rust-2021%20edition-7aa2f7?style=for-the-badge&labelColor=161a2c&logo=rust&logoColor=c8d3f5">
<img alt="built with: ratatui" src="https://img.shields.io/badge/built%20with-ratatui-73daca?style=for-the-badge&labelColor=161a2c">
<img alt="runs in: your terminal" src="https://img.shields.io/badge/runs%20in-your%20terminal-bb9af7?style=for-the-badge&labelColor=161a2c">
<img alt="keys: vim-style" src="https://img.shields.io/badge/keys-vim--style-2ac3de?style=for-the-badge&labelColor=161a2c&logo=vim&logoColor=c8d3f5">

<br><br>

<img src="docs/demo.gif" width="100%" alt="Demo: moving around people.tsv, editing a cell, adding a column with tt and naming it, deleting a column with xx, undoing and redoing with u and Ctrl-R, then filtering for zürich vip so only matching rows remain with the matching cells highlighted.">

<sub>Recorded from <code>samples/people.tsv</code> with <a href="https://github.com/charmbracelet/vhs">vhs</a> using <code>docs/demo.tape</code>.</sub>

</div>

---

## ✦ What it does

- **Lives in your terminal.** A single small binary built on
  [ratatui](https://ratatui.rs). No window, no GPU, works over SSH.
- **Vim-style keys.** `hjkl` to move, `i` / `a` to edit, `Esc` to commit,
  `:w` / `:q` / `:wq` to save and quit.
- **Columns in two keystrokes.** `tt` adds a column to the right, `xx` deletes
  the one you're on.
- **Filter as you type.** `/` then a few words: rows that don't contain every
  word disappear, and the matching cells are highlighted so the hits stand
  out. `Enter` keeps the filter while you move and edit, `Esc` clears it.
- **Paste from anywhere.** Copy cells from a spreadsheet or a TSV file and
  paste them with your terminal (`Cmd-V`, `Ctrl-Shift-V`) or `p`. They land
  at the cursor, the grid grows to fit, and `u` takes it all back.
- **Undo everything.** `u` and `Ctrl-R` undo and redo cell edits, rows and
  columns alike, and the unsaved marker clears when you undo back to the saved
  state.
- **Readable at a glance.** Each column gets its own color, the first row is
  pinned as a header, and columns size themselves to their content (capped at
  48 characters, with `…` for anything longer).
- **Knows where you are.** The status line shows the mode, file name, unsaved
  marker, row and column, and the header name of the current column.

## ⚡ Install and run

You need a terminal.

**Prebuilt binaries** for Linux (x86_64, arm64), macOS (Apple Silicon, Intel)
and Windows (x86_64) are attached to each
[GitHub release](https://github.com/danihegglin/tsvmaster3000/releases). Unpack
the archive and put `tsv` (or `tsv.exe`) on your `PATH`. To build from source
instead, you need a recent stable [Rust toolchain](https://rustup.rs).

**Try it without installing:**

```sh
git clone https://github.com/danihegglin/tsvmaster3000.git
cd tsvmaster3000
cargo run --release -- samples/people.tsv
```

**Install the `tsv` command** (into `~/.cargo/bin`):

```sh
cargo install --git https://github.com/danihegglin/tsvmaster3000
# or, from a clone:
cargo install --path .
```

**Run it:**

```sh
tsv data.tsv        # open a file
tsv new.tsv         # a path that doesn't exist yet starts empty; :w creates it
```

A file argument is required.

## ⌨ Keys

| Normal | |
|---|---|
| `h j k l`, arrows, `Tab` / `Shift-Tab` | move |
| `Enter` | move down |
| `PgUp` / `PgDn` | move 20 rows |
| `g` / `G` | first / last row |
| `0` / `$`, `Home` / `End` | first / last column |
| `i` / `a` | edit the cell (caret at start / end) |
| `x` | clear the cell |
| `o` | new row below |
| `dd` / `D` | delete the row (the header row can't be deleted) |
| `tt` | add a column to the right |
| `xx` | delete the column (the last remaining column can't be deleted) |
| `u` / `Ctrl-R` | undo / redo |
| `p` / `Ctrl-V` | paste TSV from the clipboard at the cursor (see below) |
| `/` | filter rows (see below) |
| `Esc` | clear the filter |
| `Ctrl-S` | save |
| `q` | quit (refuses if there are unsaved changes) |
| `Ctrl-C` | quit immediately, **discarding unsaved changes** |

| Insert | |
|---|---|
| `Esc` | commit |
| `Enter` | commit and move down |
| `Tab` / `Shift-Tab` | commit and keep editing the cell to the right / left (`Tab` on the last column adds a new one) |
| `↑` / `↓` | commit and keep editing the cell above / below |
| `←` `→` `Home` `End` | move the caret within the cell |
| `Backspace` / `Delete` | delete before / at the caret |

| Filter | |
|---|---|
| type | space-separated words; a row stays if every word appears in one of its cells, and cells containing a word are shown bold and underlined. Lowercase ignores case, any uppercase letter makes it case-sensitive |
| `Enter` | keep the filter: `j` / `k`, `G` and paging skip hidden rows, and edits work as usual |
| `Esc` | clear the filter |

The header row always stays. The row the cursor is on stays visible even if an
edit (or `o`) means it no longer matches, until you move off it.

| Command | |
|---|---|
| `:w` | save |
| `:q` / `:q!` | quit / quit discarding changes |
| `:wq` / `:wq!` / `:x` | save and quit (if the save fails, it stays open) |
| `:N` / `:$` | go to row N (the number in the left gutter) / the last row |
| `:h` | show a key summary in the status line |

### Pasting

Pasted text is read as TSV: lines become rows and tabs separate cells. The
block overwrites cells starting at the cursor, and rows and columns are added
when it reaches past the edge. The whole paste is one undo step.

- **Your terminal's paste** (`Cmd-V` on macOS, `Ctrl-Shift-V` on most Linux
  terminals) works in every mode and also over SSH. While editing a cell,
  plain text goes in at the caret; text with tabs or line breaks saves the
  cell and is pasted as a block.
- **`p` or `Ctrl-V`** in normal mode read the system clipboard directly. Use
  it where the terminal's paste doesn't arrive as a block (for example the
  classic Windows console). It needs a local desktop session: on Linux that is
  X11 or XWayland.
- While a filter is active only single-row pastes are allowed, so hidden rows
  are never overwritten. `Esc` clears the filter.

## 📝 Good to know

- The whole file is read into memory when it opens.
- Short rows are padded with empty cells to the widest row, and the file is
  saved with `\n` line endings and a trailing newline.
- Tabs or newlines typed into a cell are saved as spaces, so the file stays
  valid TSV.

## 🛠 Building

```sh
cargo build --release       # binary at target/release/tsv
cargo test                  # tests that drive real key sequences
vhs docs/demo.tape          # re-record the README demo (needs vhs and a release build)
```

CI (`.github/workflows/release.yml`) tests and packages every push and pull
request for Linux, macOS and Windows; the archives are kept as workflow
artifacts. Pushing a tag like `v0.2.0` also publishes them, with a
`SHA256SUMS` file, as a GitHub release:

```sh
git tag v0.2.0 && git push origin v0.2.0
```
