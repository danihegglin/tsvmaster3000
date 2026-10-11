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

- **Lives in your terminal.** One small binary built on
  [ratatui](https://ratatui.rs). Works over SSH.
- **Vim-style keys.** `hjkl` to move, `i` / `a` to edit, `:wq` to save and quit.
- **Filter as you type.** `/` and a few words hides non-matching rows and
  highlights the hits.
- **Paste from anywhere.** Cells copied from a spreadsheet or TSV land at the
  cursor and the grid grows to fit.
- **Undo everything.** `u` / `Ctrl-R` cover cells, rows and columns.
- **Readable at a glance.** A color per column, a pinned header row, and
  columns sized to their content.

## ⚡ Install

Grab a prebuilt binary for Linux, macOS or Windows from the
[releases](https://github.com/danihegglin/tsvmaster3000/releases) and put
`tsv` on your `PATH`, or build it with a stable
[Rust toolchain](https://rustup.rs):

```sh
cargo install --git https://github.com/danihegglin/tsvmaster3000
```

Then:

```sh
tsv data.tsv        # a path that doesn't exist yet starts empty; :w creates it
```

## ⌨ Keys

| Normal | |
|---|---|
| `h j k l`, arrows, `Tab` / `Shift-Tab` | move |
| `Enter` / `PgUp` / `PgDn` | move down / 20 rows up / 20 rows down |
| `g` / `G`, `0` / `$` | first / last row, first / last column |
| `i` / `a` | edit the cell (caret at start / end) |
| `x` | clear the cell |
| `o` / `dd` | add a row below / delete the row (`D` also deletes; not the header) |
| `tt` / `xx` | add a column to the right / delete the column |
| `u` / `Ctrl-R` | undo / redo |
| `p` / `Ctrl-V` | paste from the system clipboard |
| `/` / `Esc` | filter rows / clear the filter |
| `Ctrl-S` / `q` | save / quit (refuses with unsaved changes) |
| `Ctrl-C` | quit immediately, **discarding unsaved changes** |

| Insert | |
|---|---|
| `Esc` / `Enter` | commit / commit and move down |
| `Tab` / `Shift-Tab`, `↑` / `↓` | commit and keep editing the next cell (`Tab` past the last column adds one) |
| `←` `→` `Home` `End`, `Backspace` `Delete` | edit within the cell |

| Command | |
|---|---|
| `:w`, `:q`, `:q!`, `:wq`, `:x` | save / quit / discard and quit / save and quit |
| `:N` / `:$` | go to row N (as in the gutter) / the last row |
| `:h` | key summary in the status line |

**Filtering.** A row stays if every space-separated word appears in one of its
cells. Lowercase ignores case; any uppercase letter makes it case-sensitive.
`Enter` keeps the filter while you move and edit; the header and the row
you're on always stay visible.

**Pasting.** Lines become rows and tabs separate cells; the block overwrites
from the cursor as a single undo step. Your terminal's paste (`Cmd-V`,
`Ctrl-Shift-V`) works in every mode, including over SSH. `p` / `Ctrl-V` read
the clipboard directly for terminals where that doesn't work, and need a local
desktop session (X11 or XWayland on Linux). While filtering, only single-row
pastes are allowed.
