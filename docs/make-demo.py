#!/usr/bin/env python3
"""Render docs/demo.svg: an animated mockup of the editor for the README.

It isn't a screen recording. Each frame is drawn from samples/people.tsv with
the editor's own palette and layout rules (src/editor.rs), and the frames are
cycled with CSS like a GIF. Re-run after changing the look:

    python3 docs/make-demo.py
    python3 docs/make-demo.py --frame 3 out.svg   # one still frame, for checking
"""

import sys
from html import escape
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Palette, as in src/editor.rs.
BG, BG_DARK, BG_STRIPE, BG_HIGHLIGHT = "#161a2c", "#11141f", "#191e33", "#21283f"
HEADER_TOP, HEADER_BOTTOM, BAR_TOP, BLACK = "#1d2340", "#171c30", "#151928", "#0d0f18"
FG, FG_DARK, FG_GUTTER, COMMENT, GRID = "#c8d3f5", "#9aa8d6", "#363f63", "#56608f", "#1f2539"
BLUE, BLUE1, BLUE5, ICE, TEAL, MAGENTA = "#7aa2f7", "#2ac3de", "#89ddff", "#b4f9f8", "#73daca", "#bb9af7"
MODE = {"NORMAL": BLUE, "INSERT": TEAL, "VISUAL": MAGENTA, "COMMAND": BLUE1, "FILTER": ICE}

FONT, CW, RH, PAD = 13, 7.8, 23, 6
WIN_W, TITLE_H, DATA_ROWS = 920, 30, 10
GW = 5 * CW  # gutter: 3 digits + 2
W = WIN_W
H = RH * (2 + DATA_ROWS) + 2 * RH  # column bar, header, rows, status, cmdline
FRAME_SECS = 1.9


def load():
    lines = (ROOT / "samples/people.tsv").read_text().rstrip("\n").split("\n")
    rows = [l.split("\t") for l in lines]
    ncols = max(map(len, rows))
    return [r + [""] * (ncols - len(r)) for r in rows]


def col_name(c):
    s = ""
    while True:
        s = chr(65 + c % 26) + s
        if c < 26:
            return s
        c = c // 26 - 1


def fuzzy(query, s):
    icase = not any(ch.isupper() for ch in query)
    for term in query.split():
        it = iter(s.lower() if icase else s)
        if not all(any(ch == w for ch in it) for w in (term.lower() if icase else term)):
            return False
    return True


def numeric(s):
    s = s.strip()
    try:
        float(s)
        return bool(s) and s[-1].isdigit()
    except ValueError:
        return False


def truncate(s, n):
    return s if len(s) <= n else (s[: n - 1] + "…" if n > 0 else "")


def fit(table):
    return [min(max(3, *(len(r[c]) for r in table)), 40) for c in range(len(table[0]))]


class Svg:
    def __init__(self):
        self.out = []

    def rect(self, x, y, w, h, fill, r=0, extra=""):
        rr = f' rx="{r}"' if r else ""
        self.out.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{max(w, 0):.1f}" height="{h:.1f}"{rr} fill="{fill}"{extra}/>')

    def ring(self, x, y, w, h, r, t, color, opacity=1.0):
        self.out.append(
            f'<rect x="{x + t / 2:.1f}" y="{y + t / 2:.1f}" width="{w - t:.1f}" height="{h - t:.1f}" rx="{r}" '
            f'fill="none" stroke="{color}" stroke-width="{t}" stroke-opacity="{opacity}"/>'
        )

    def text(self, s, x, y, color, bold=False, anchor="start", opacity=1.0):
        if not s:
            return
        b = ' font-weight="700"' if bold else ""
        a = f' text-anchor="{anchor}"' if anchor != "start" else ""
        o = f' fill-opacity="{opacity}"' if opacity != 1.0 else ""
        self.out.append(f'<text x="{x:.1f}" y="{y + RH / 2 + 4.5:.1f}" fill="{color}"{b}{a}{o}>{escape(s)}</text>')


def frame(table, st):
    """Draw one editor state; mirrors Editor::build_frame / build_status."""
    s = Svg()
    mode = st.get("mode", "NORMAL")
    accent = MODE[mode]
    widths = st.get("widths") or fit(table)
    cur_r, cur_c = st["cursor"]
    query = st.get("filter", "")
    edit = st.get("edit")

    # Rows shown: header plus matches when filtering.
    data = range(1, len(table))
    if query.strip():
        data = [r for r in data if any(c and fuzzy(query, c) for c in table[r])]
    shown = [0] + list(data)
    ncols = len(table[0])

    s.rect(0, 0, W, H, BG)
    colw = lambda c: max(widths[c], len(edit) + 1 if edit is not None and c == cur_c else 0) * CW + 2 * PAD
    cols, x = [], GW
    for c in range(ncols):
        cols.append((c, x, x + colw(c)))
        x += colw(c)
        if x >= W:
            break
    grid_right = min(x, W)

    # Column bar.
    s.rect(0, 0, W, RH, "url(#bar)")
    for c, x0, x1 in cols:
        active = c == cur_c
        if active:
            s.rect(x0 + 3, 3, x1 - x0 - 6, RH - 6, accent, r=(RH - 6) / 2, extra=' fill-opacity="0.18"')
        s.text(col_name(c), (x0 + x1) / 2, 0, accent if active else COMMENT, active, "middle")

    rows = [(0, RH)] + [(r, RH * (2 + i)) for i, r in enumerate(shown[1:DATA_ROWS + 1])]
    grid_bottom = rows[-1][1] + RH
    for d, (r, y) in enumerate(rows):
        if r == 0:
            s.rect(GW, y, grid_right - GW, RH, "url(#head)")
        elif r == cur_r:
            s.rect(GW, y, grid_right - GW, RH, BG_HIGHLIGHT)
        elif d % 2 == 1:
            s.rect(GW, y, grid_right - GW, RH, BG_STRIPE)
        if r == cur_r:
            s.rect(1, y + 4, 3, RH - 8, accent, r=1.5)
        s.text(f"{r + 1:>3}", CW, y, accent if r == cur_r else FG_GUTTER, r == cur_r)
        for c, x0, x1 in cols:
            cell = table[r][c]
            if r and query.strip() and not fuzzy(query, cell):
                cell = ""
            cursor = (r, c) == (cur_r, cur_c)
            if cursor:
                s.rect(x0, y, x1 - x0, RH, accent, extra=' fill-opacity="0.14"')
            if not cell or (cursor and edit is not None):
                continue
            n = int((x1 - x0 - 2 * PAD) / CW + 1e-6)
            num = r > 0 and numeric(cell)
            color = BLUE5 if r == 0 else TEAL if num else FG
            if num:
                s.text(truncate(cell, n), x1 - PAD, y, color, anchor="end")
            else:
                s.text(truncate(cell, n), x0 + PAD, y, color, r == 0)

    for _, _, x1 in cols:
        s.rect(x1 - 1, RH, 1, grid_bottom - RH, GRID)
    s.rect(GW - 1, RH, 1, grid_bottom - RH, GRID)
    s.rect(GW, 2 * RH - 1, grid_right - GW, 1, f"url(#line-{mode})")

    # Cursor: glow, ring, insert overlay.
    ys = {r: y for r, y in rows}
    xs = {c: (x0, x1) for c, x0, x1 in cols}
    if cur_r in ys and cur_c in xs:
        y, (x0, x1) = ys[cur_r], xs[cur_c]
        cw = min(x1, W) - x0
        s.ring(x0 - 4, y - 4, cw + 8, RH + 8, 8, 2, accent, 0.07)
        s.ring(x0 - 2, y - 2, cw + 4, RH + 4, 6, 2, accent, 0.2)
        if edit is not None:
            s.rect(x0, y, cw, RH, BG_DARK, r=4)
            s.text(edit, x0 + PAD, y, FG)
            s.rect(x0 + PAD + len(edit) * CW, y + 3, 2, RH - 6, accent)
        s.ring(x0, y, cw, RH, 4, 1.5, accent)

    # Status bar.
    sy = H - 2 * RH
    s.rect(0, sy, W, 2 * RH, BG_DARK)
    s.rect(0, sy, W, 1, GRID)
    pill = RH - 8
    lw = (len(mode) + 2) * CW
    s.rect(CW / 2, sy + 4, lw, pill, accent, r=pill / 2)
    s.text(mode, CW * 1.5, sy, BLACK, True)
    name = "samples/people.tsv"
    nx = CW / 2 + lw + CW
    s.text(name, nx, sy, FG_DARK)
    if st.get("dirty"):
        s.text("●", nx + (len(name) + 1) * CW, sy, accent)
    ref = f"{col_name(cur_c)}{cur_r + 1}"
    zw = (len(ref) + 2) * CW
    zx = W - zw - CW / 2
    s.rect(zx, sy + 4, zw, pill, accent, r=pill / 2, extra=' fill-opacity="0.18"')
    s.text(ref, zx + CW, sy, accent, True)
    pos = f"row {cur_r + 1}/{len(table)} · col {cur_c + 1}/{ncols}"
    s.text(pos, zx - CW, sy, COMMENT, anchor="end")
    extra = ""
    if query.strip():
        if mode != "FILTER":
            extra += f"filter: {query}  "
        extra += f"{len(shown) - 1} of {len(table) - 1} rows"
    s.text(extra, zx - CW - (len(pos) + 2) * CW, sy, FG_DARK, anchor="end")

    cy = H - RH
    if mode == "FILTER":
        line = f"filter: {query}"
        s.text(line, CW, cy, FG)
        s.rect(CW + len(line) * CW, cy + 3, 2, RH - 6, accent)
    elif st.get("msg"):
        s.text(st["msg"], CW, cy, FG)
    else:
        s.text(f"{ref} ", CW, cy, COMMENT, True)
        s.text(table[cur_r][cur_c], CW + (len(ref) + 2) * CW, cy, FG)

    # Keystroke caption, like a screencast overlay.
    keys = st.get("keys")
    if keys:
        caps = keys.split(" ")
        kx = W - 16
        ky = sy - RH - 14
        widths_ = [max(len(k), 1) * CW + 14 for k in caps]
        total = sum(widths_) + 6 * (len(caps) - 1) + 16
        s.rect(kx - total, ky - 6, total, RH + 12, BLACK, r=10, extra=' fill-opacity="0.85"')
        s.ring(kx - total, ky - 6, total, RH + 12, 10, 1, accent, 0.5)
        x = kx - total + 8
        for k, kw in zip(caps, widths_):
            s.rect(x, ky, kw, RH, BG_HIGHLIGHT, r=5)
            s.rect(x, ky + RH - 3, kw, 3, accent, r=1.5, extra=' fill-opacity="0.6"')
            s.text(k, x + kw / 2, ky - 1, FG, True, "middle")
            x += kw + 6
    return "\n".join(s.out)


def main():
    table = load()
    wide = fit(table)
    toronto = "Toronto, Canada"
    wider = list(wide)
    wider[2] = max(wider[2], len(toronto))
    edited = [list(r) for r in table]
    edited[3][2] = toronto
    no_age = [r[:3] + r[4:] for r in edited]
    no_age_w = wider[:3] + wider[4:]
    size = sum(len("\t".join(r).encode()) + 1 for r in edited)
    loaded = f'"samples/people.tsv" {len(table)} rows × {len(table[0])} cols, 0.0 MB loaded in 180µs'

    frames = [
        (table, dict(cursor=(1, 1), msg=loaded)),
        (table, dict(cursor=(3, 2), keys="j j l")),
        (table, dict(cursor=(3, 2), mode="INSERT", edit="Toronto, Can", keys="a , ␣ C a n")),
        (edited, dict(cursor=(3, 2), widths=wider, dirty=True, keys="a d a Esc")),
        (edited, dict(cursor=(4, 2), widths=wider, dirty=True, mode="FILTER", filter="zri", keys="␣ ␣ z r i")),
        (edited, dict(cursor=(4, 2), widths=wider, dirty=True, filter="zri", keys="Enter")),
        (edited, dict(cursor=(3, 3), widths=wider, dirty=True, keys="Esc k l")),
        (no_age, dict(cursor=(3, 3), widths=no_age_w, dirty=True, keys="x x",
                      msg="Deleted column D “age” · ⌘Z to undo")),
        (edited, dict(cursor=(3, 3), widths=wider, dirty=True, keys="⌘ Z", msg="undo: 1 change")),
        (edited, dict(cursor=(3, 3), widths=wider, keys="⌘ S",
                      msg=f'"samples/people.tsv" {len(table)} rows, {size} bytes written in 95µs')),
    ]

    out = ROOT / "docs/demo.svg"
    if len(sys.argv) == 4 and sys.argv[1] == "--frame":
        frames = [frames[int(sys.argv[2])]]
        out = Path(sys.argv[3])
    n = len(frames)
    total = n * FRAME_SECS
    css = [f".f{{opacity:0;animation:{total:.1f}s step-end infinite}}"]
    for i in range(n):
        a, b = 100 * i / n, 100 * (i + 1) / n
        steps = [f"0%{{opacity:{1 if i == 0 else 0}}}"]
        if i:
            steps.append(f"{a:.2f}%{{opacity:1}}")
        steps.append(f"{b:.2f}%{{opacity:0}}")
        css.append(f"@keyframes k{i}{{{''.join(steps)}}}.f{i}{{animation-name:k{i}}}")
    # Without animation (reduced motion, some viewers) show the first frame.
    css.append(f"@media (prefers-reduced-motion:reduce){{.f{{animation:none}}.f0{{opacity:1}}}}")

    lines = "".join(
        f'<linearGradient id="line-{m}"><stop offset="0" stop-color="{c}" stop-opacity="0.7"/>'
        f'<stop offset="1" stop-color="{c}" stop-opacity="0.05"/></linearGradient>'
        for m, c in MODE.items()
    )
    pad = 24
    ow, oh = W + 2 * pad, H + TITLE_H + 2 * pad
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{ow}" height="{oh}" viewBox="0 0 {ow} {oh}" '
        f'font-family="SF Mono, Menlo, Consolas, \'DejaVu Sans Mono\', monospace" font-size="{FONT}">',
        f"<style>{''.join(css)}</style>",
        "<defs>",
        f'<linearGradient id="bar" x2="0" y2="1"><stop offset="0" stop-color="{BAR_TOP}"/><stop offset="1" stop-color="{BG_DARK}"/></linearGradient>',
        f'<linearGradient id="head" x2="0" y2="1"><stop offset="0" stop-color="{HEADER_TOP}"/><stop offset="1" stop-color="{HEADER_BOTTOM}"/></linearGradient>',
        lines,
        f'<filter id="shadow" x="-10%" y="-10%" width="120%" height="130%"><feDropShadow dx="0" dy="10" stdDeviation="12" flood-color="#000" flood-opacity="0.45"/></filter>',
        f'<clipPath id="win"><rect x="{pad}" y="{pad}" width="{W}" height="{H + TITLE_H}" rx="10"/></clipPath>',
        "</defs>",
        f'<rect x="{pad}" y="{pad}" width="{W}" height="{H + TITLE_H}" rx="10" fill="{BG_DARK}" filter="url(#shadow)"/>',
        f'<g clip-path="url(#win)">',
        f'<rect x="{pad}" y="{pad}" width="{W}" height="{TITLE_H}" fill="#0f1220"/>',
    ]
    for i, color in enumerate(["#f7768e", "#e0af68", "#9ece6a"]):
        parts.append(f'<circle cx="{pad + 18 + i * 20}" cy="{pad + TITLE_H / 2}" r="6" fill="{color}"/>')
    parts.append(
        f'<text x="{pad + W / 2}" y="{pad + TITLE_H / 2 + 4.5}" fill="{COMMENT}" text-anchor="middle">'
        f"people.tsv — tsv</text>"
    )
    parts.append(f'<g transform="translate({pad},{pad + TITLE_H})">')
    for i, (t, st) in enumerate(frames):
        parts.append(f'<g class="f f{i}">{frame(t, st)}</g>')
    parts += ["</g>", "</g>", "</svg>"]
    out.write_text("\n".join(parts) + "\n")
    print(f"wrote {out}: {n} frames, {total:.1f}s loop, {out.stat().st_size // 1024} KB")


if __name__ == "__main__":
    main()
