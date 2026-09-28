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

<b>▓▒░ a blazing-fast TSV editor · vim keys · fuzzy filter · Tokyo Night glow ░▒▓</b>

<br><br>

<img alt="rust: 2024 edition" src="https://img.shields.io/badge/rust-2024%20edition-7aa2f7?style=for-the-badge&labelColor=161a2c&logo=rust&logoColor=c8d3f5">
<img alt="built with: gpui" src="https://img.shields.io/badge/built%20with-gpui-73daca?style=for-the-badge&labelColor=161a2c">
<img alt="platform: macOS" src="https://img.shields.io/badge/platform-macOS-bb9af7?style=for-the-badge&labelColor=161a2c&logo=apple&logoColor=c8d3f5">
<img alt="keys: vim" src="https://img.shields.io/badge/keys-vim-2ac3de?style=for-the-badge&labelColor=161a2c&logo=vim&logoColor=c8d3f5">

<br><br>

<img src="docs/demo.svg" width="100%" alt="Demo: moving around people.tsv, editing a cell while its column widens, fuzzy-filtering for Zürich so non-matching rows disappear, deleting a column with xx and bringing it back with ⌘Z, then saving.">

<sub>Animated mockup drawn from <code>samples/people.tsv</code> by <code>docs/make-demo.py</code>, not a screen recording.</sub>

</div>

---

## ✦ What it does

- **Fast on big files.** One SIMD pass indexes the file; rows stay as
  byte ranges into the buffer until you touch them. Only the visible cells are
  ever drawn.
- **Real vim keys.** Counts, operators and motions (`3dd`, `y}`, `d$`), visual
  and visual-line mode, `.` repeat, registers that talk to the system
  clipboard, `:s`, `:sort`, `:w`.
- **Fuzzy filter.** `Space` `Space`, type a few letters: rows without a match
  vanish, non-matching cells go blank, and edits only touch what you can see.
- **Rows and columns without ceremony.** `tt` / `xx` add and delete columns,
  `⌘+` / `⌘−` too, `⌥` + arrow adds in any direction, and every change says
  what it did and how to undo it.
- **Undo everything.** `u` / `⌘Z` and `Ctrl-R` / `⌘⇧Z` cover cells, rows and
  columns alike.
- **Columns that grow while you type,** a pinned header row, auto-fit widths,
  and a mode-tinted Tokyo Night theme with a glowing cursor.

## ⚡ Install and run

You need macOS and a recent stable [Rust toolchain](https://rustup.rs). gpui
also targets Linux, but that's untested here.

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
tsv                 # empty table; save with :w name.tsv
```

The first build compiles gpui and takes a few minutes; after that it's quick.

## 🕹 60-second tour

| Do this | Keys |
|---|---|
| Move | `h j k l` or arrows, `gg` / `G`, `w` / `b` jump between filled cells |
| Edit a cell | `i` / `a` / `Enter`, type, `Esc` (or `Tab` / `Enter` to move on) |
| Add / delete a row | `o` / `dd` (or `⌥↓` / `⌘⌫`) |
| Add / delete a column | `tt` / `xx` (or `⌘+` / `⌘−`) |
| Filter rows | `Space` `Space`, type, `Enter` to keep, `Esc` to clear |
| Search / replace | `/regex`, `n` / `N`, `:%s/old/new/g` |
| Undo / redo | `u` / `Ctrl-R` or `⌘Z` / `⌘⇧Z` |
| Save / quit | `⌘S` / `:w`, `⌘Q` / `:q` |

## ⌨ All keys

<details>
<summary><b>Normal, insert and ex mode</b></summary>

<br>

| Normal | |
|---|---|
| `h j k l`, arrows, `w`/`b` (next/prev filled cell), `0 ^ $ \|` | move |
| `gg` `G` `{n}G` `H M L` `^d ^u ^f ^b ^e ^y` `zz zt zb` | jump / scroll |
| `{` `}` | jump to the edge of a block in this column (like Ctrl+Arrow in Excel) |
| `i` `a` `Enter` / `I` `A` | edit the cell (cursor at start/end) / first/last cell |
| `s` `cc` `C` | replace the cell |
| `o` `O` | new row below/above |
| `x` `D` | cut cell / cut to end of row |
| `tt` / `xx` | add a column to the right / delete the column |
| `dd` `yy` `d{motion}` `y{motion}` | delete/yank rows (vertical motions) or cells (horizontal) |
| `dc` `yc` | delete/yank column |
| `p` `P` | paste (uses the system clipboard if it changed outside the editor) |
| `u` `^r` `.` | undo, redo, repeat last change |
| `~` `^a` `^x` | toggle case, increment/decrement number |
| `>` `<` `=` | widen / narrow / auto-fit column |
| `v` `V` | cell block / row selection, then `d y c p ~ u U ^a >` |
| `/` `?` `n` `N` `*` `#` | regex search (smartcase) |
| `Space` `Space` | fuzzy filter (fzf-style, space-separated terms, smartcase): rows without a match are hidden and non-matching cells blanked. `Tab`/`S-Tab` jump between matches, `Enter` keeps the filter (then `n`/`N` jump), `Esc` clears it. While filtering, row motions and row edits (`dd`, `V…d`, `:s`, …) only touch the rows you can see; column edits still cover every row |

Insert mode: `Esc` commits, `Enter`/`Tab`/`Shift-Tab` commit and move (Enter at
the last row adds a row). `^w ^u ^k` and arrows work as usual.

| Ex | |
|---|---|
| `:w [file]` `:q[!]` `:wq` `:x` `:e[!] file` | files |
| `:N` `:$` | go to row |
| `:[range]s/pat/rep/[gi]` | substitute in cells (`%` = all rows) |
| `:[range]sort[!] [r]` | stable sort by cursor column (numbers are compared as numbers; `r` compares raw text) |
| `:[range]d` `:[range]y` | delete/yank rows |
| `:ic [n]` `:ac [n]` `:dc [n]` | insert column before/after, delete column |
| `:set header!` `:set rnu` `:noh` `:fit` `:width N` | view |

`⌘S` saves, `⌘Q` quits. `⌘Z` / `⌘⇧Z` (or `⌘Y`) undo / redo in every mode.
The first row is pinned as a header (`:set noheader` to turn that off).

</details>

<details>
<summary><b>Rows and columns</b></summary>

<br>

| | Column | Row |
|---|---|---|
| add | `⌘+` or `⌥→` right, `⌥←` left | `⌥↓` below, `⌥↑` above |
| delete | `⌘−` or `⌘⇧⌫` | `⌘⌫` |
| select | click its letter (shift-click extends) | click its number (shift-click extends) |
| add at the end | click `+` after the last column | click `+` below the last row |

Adding or deleting works on the cursor's row/column, or on every selected
row/column in visual mode. The message line says what changed. `⌘Enter` /
`⌘⇧Enter` also add a row, even while editing a cell (in a cell, the
`⌥`/`⌘-Backspace` keys edit text as usual). The vim ways still work: `o` `O`
`dd` `tt` `xx` `dc` `:ic` `:ac` `:dc`.

</details>

## 🚀 Why it's fast

- **Loading**: the file is read into one buffer and indexed with a single SIMD
  pass (`memchr2` over tabs/newlines). No per-cell allocations; each row is a
  byte range into the buffer until you edit it.
- **Rendering**: one `canvas` paints only the visible cells. The work per frame
  depends on the window size, not the file size, and gpui caches shaped text
  lines between frames.
- **Saving**: untouched runs of contiguous rows are written back with one
  `write` each, then an atomic rename.
- **Search / `:s`**: regex prefilter on the whole line before looking at cells.

## 🛠 Building and hacking

The default `runtime-shaders` feature compiles the Metal shaders at startup, so
you don't need Xcode's Metal toolchain. To precompile them instead, run
`xcodebuild -downloadComponent MetalToolchain` and then build with
`--no-default-features`.

```sh
cargo test                  # unit tests plus editor tests that drive real key sequences
python3 docs/make-demo.py   # regenerate the README demo after changing the look
```
