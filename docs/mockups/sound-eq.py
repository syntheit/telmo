"""telmo Sound EQ mockups: 90x22 cell screens rendered to a standalone HTML page."""
import html, json, math, os

W, H = 90, 22
C = {"fg": "#c0caf5", "dim": "#737aa2", "faint": "#3b4261", "border": "#3b4261", "blue": "#7aa2f7",
     "cyan": "#7dcfff", "green": "#9ece6a", "yellow": "#e0af68", "red": "#f7768e", "magenta": "#bb9af7",
     "orange": "#ff9e64", "sel": "#283457", "modal": "#13141f", "spec": "#2d3352"}


class Grid:
    def __init__(self):
        self.g = [[(" ", C["fg"], None, False) for _ in range(W)] for _ in range(H)]

    def put(self, r, c, cells):
        for i, cell in enumerate(cells):
            if 0 <= c + i < W and 0 <= r < H:
                self.g[r][c + i] = cell

    def html(self):
        out = []
        for row in self.g:
            line, run, key = "", "", None
            def flush():
                nonlocal line, run
                if run:
                    fg, bg, b = key
                    style = f"color:{fg}" + (f";background:{bg}" if bg else "") + (";font-weight:700" if b else "")
                    line += f'<span style="{style}">{html.escape(run)}</span>'
                    run = ""
            for ch, fg, bg, b in row:
                k = (fg, bg, b)
                if k != key:
                    flush()
                    key = k
                run += ch
            flush()
            out.append(line)
        return "\n".join(out)

    def text(self):
        return "\n".join("".join(ch for ch, *_ in row).rstrip() for row in self.g)


def T(s, c="fg", bold=False, bg=None):
    return [(ch, C.get(c, c), bg, bold) for ch in s]


def pad(cells, w):
    return cells[:w] + T(" " * max(0, w - len(cells)))


def spread(left, right, w):
    return left + T(" " * max(1, w - len(left) - len(right))) + right


def keys(pairs, gap=2):
    cells = T(" ")
    for i, (k, label) in enumerate(pairs):
        if i:
            cells += T(" " * gap)
        cells += T(k, "blue") + T(" " + label, "dim")
    return cells


def box(title, lines, w, h, active=False, right=None):
    border = "blue" if active else "border"
    top = T("╭─", border) + T(f" {title} ", "blue" if active else "fg", True)
    tail = (T(" ") + right + T(" ─", border)) if right else []
    top += T("─" * (w - len(top) - len(tail) - 1), border) + tail + T("╮", border)
    rows = [top]
    lines = list(lines) + [[]] * max(0, h - 2 - len(lines))
    for cells in lines[: h - 2]:
        rows.append(T("│", border) + pad(cells, w - 2) + T("│", border))
    rows.append(T("╰" + "─" * (w - 2) + "╯", border))
    return rows


def place(g, r, c, rows):
    for i, cells in enumerate(rows):
        g.put(r + i, c, cells)


# ---------------------------------------------------------------- filters (RBJ cookbook, 48 kHz)

FS = 48000.0


def biquad(kind, f0, gain, q):
    A = 10 ** (gain / 40)
    w0 = 2 * math.pi * f0 / FS
    cw, sw = math.cos(w0), math.sin(w0)
    alpha = sw / (2 * q)
    if kind == "PK":
        b = [1 + alpha * A, -2 * cw, 1 - alpha * A]
        a = [1 + alpha / A, -2 * cw, 1 - alpha / A]
    elif kind == "LSC":
        s = 2 * math.sqrt(A) * alpha
        b = [A * ((A + 1) - (A - 1) * cw + s), 2 * A * ((A - 1) - (A + 1) * cw), A * ((A + 1) - (A - 1) * cw - s)]
        a = [(A + 1) + (A - 1) * cw + s, -2 * ((A - 1) + (A + 1) * cw), (A + 1) + (A - 1) * cw - s]
    elif kind == "HSC":
        s = 2 * math.sqrt(A) * alpha
        b = [A * ((A + 1) + (A - 1) * cw + s), -2 * A * ((A - 1) + (A + 1) * cw), A * ((A + 1) + (A - 1) * cw - s)]
        a = [(A + 1) - (A - 1) * cw + s, 2 * ((A - 1) - (A + 1) * cw), (A + 1) - (A - 1) * cw - s]
    else:  # HP
        b = [(1 + cw) / 2, -(1 + cw), (1 + cw) / 2]
        a = [1 + alpha, -2 * cw, 1 - alpha]
    return b, a


def mag_db(filters, f):
    total = 0.0
    w = 2 * math.pi * f / FS
    for kind, f0, gain, q in filters:
        b, a = biquad(kind, f0, gain, q)
        z1, z2 = complex(math.cos(-w), math.sin(-w)), complex(math.cos(-2 * w), math.sin(-2 * w))
        h = (b[0] + b[1] * z1 + b[2] * z2) / (a[0] + a[1] * z1 + a[2] * z2)
        total += 20 * math.log10(abs(h))
    return total


# ---------------------------------------------------------------- presets (from the research)

P = {
    "Flat": (0, []),
    "Speakers +": (-3, [("PK", 160, 3, 0.7)]),
    "Speakers ++": (-4.5, [("PK", 170, 4.5, 0.7), ("HP", 40, 0, 0.7)]),
    "Bass boost": (-4, [("LSC", 90, 4, 0.7), ("PK", 200, 1, 1.0)]),
    "Vocal": (-2.5, [("PK", 250, -2, 1.0), ("PK", 3000, 2.5, 1.0), ("HSC", 8000, 1.5, 0.7)]),
    "Late night": (-3, [("LSC", 120, -3, 0.7), ("PK", 3000, 1.5, 0.8), ("HSC", 9000, -2, 0.7)]),
    "Your EarFun EQ": (-10, [("PK", 31.5, 8, 1.4), ("PK", 63, 7, 1.4), ("PK", 125, 5, 1.4)]),
    "As is": (0, []),
    "+ Sub-bass": (-2, [("LSC", 100, 2, 0.7)]),
    "+ Sub-bass ++": (-3, [("LSC", 100, 3, 0.7)]),
    "Sub focus": (-2.5, [("PK", 50, 2.5, 0.8)]),
    "Warm": (-3, [("LSC", 150, 2.5, 0.7), ("PK", 3500, -1, 1.0)]),
}

LISTS = {
    "speakers": ["Speakers +", "Speakers ++", "Flat", "Bass boost", "Vocal", "Late night"],
    "earfun": ["Flat", "Your EarFun EQ", "Bass boost", "Vocal", "Late night"],
    "zerored": ["As is", "+ Sub-bass", "+ Sub-bass ++", "Sub focus", "Warm", "Late night"],
}


def describe(name):
    pre, filters = P[name]
    if not filters:
        return T("No filters: the sound as the device makes it.", "dim")
    names = {"PK": "", "LSC": "shelf ", "HSC": "shelf ", "HP": "cut below "}
    parts = []
    for kind, f0, gain, q in filters:
        hz = f"{f0/1000:g}k" if f0 >= 1000 else f"{f0:g}"
        parts.append(f"{names[kind]}{hz} Hz" + (f" {gain:+g}" if kind != "HP" else ""))
    return T(" · ".join(parts), "dim")


# ---------------------------------------------------------------- curve

SPECTRUM = [0.35, 0.55, 0.7, 0.8, 0.75, 0.62, 0.58, 0.5, 0.47, 0.42, 0.38, 0.36, 0.33, 0.3, 0.26, 0.22, 0.18, 0.12]
BARS = " ▁▂▃▄▅▆▇"


def curve(filters, w, h, color="blue", on=True, spectrum=True):
    """Response plot: dB axis on the left, log-frequency axis below, braille curve, faint live spectrum behind."""
    axis_w = 5
    pw, ph = w - axis_w, h - 1
    fmin, fmax, dbmax = 20.0, 20000.0, 10.0
    dots = [[False] * (pw * 2) for _ in range(ph * 4)]
    prev = None
    for x in range(pw * 2):
        f = fmin * (fmax / fmin) ** (x / (pw * 2 - 1))
        db = mag_db(filters, f) if on else 0.0
        y = round((dbmax - max(-dbmax, min(dbmax, db))) / (2 * dbmax) * (ph * 4 - 1))
        lo, hi = (y, y) if prev is None else (min(prev, y), max(prev, y))
        for yy in range(lo, hi + 1):
            dots[yy][x] = True
        prev = y
    zero_row = round((ph * 4 - 1) / 2) // 4
    rows = []
    labels = {0: "+10", zero_row // 2: " +5", zero_row: "  0", (zero_row + ph - 1) // 2: " -5", ph - 1: "-10"}
    for r in range(ph):
        cells = T(f"{labels.get(r, ''):>3} ", "dim") + T("┤" if r in labels else "│", "faint")
        for cx in range(pw):
            bits = 0
            for dy, dx, bit in [(0, 0, 0x01), (1, 0, 0x02), (2, 0, 0x04), (0, 1, 0x08), (1, 1, 0x10), (2, 1, 0x20), (3, 0, 0x40), (3, 1, 0x80)]:
                if dots[r * 4 + dy][cx * 2 + dx]:
                    bits |= bit
            if bits:
                cells += T(chr(0x2800 + bits), color if on else "dim", True)
                continue
            # live spectrum in the bottom rows, faint
            lvl = SPECTRUM[min(len(SPECTRUM) - 1, cx * len(SPECTRUM) // pw)]
            height = lvl * 2.2
            from_bottom = ph - 1 - r
            if spectrum and from_bottom < 2 and height > from_bottom:
                frac = min(1.0, height - from_bottom)
                cells += T(BARS[max(1, round(frac * 7))], "spec")
            elif r == zero_row:
                cells += T("─", "faint")
            else:
                cells += T(" ")
        rows.append(cells)
    ticks = [(20, "20"), (50, "50"), (100, "100"), (200, "200"), (500, "500"), (1000, "1k"), (2000, "2k"), (5000, "5k"), (10000, "10k")]
    axis = [(" ", C["dim"], None, False)] * w
    for f, label in ticks:
        x = axis_w + round(math.log(f / fmin) / math.log(fmax / fmin) * (pw - 1))
        for i, ch in enumerate(label):
            if x + i < w:
                axis[x + i] = (ch, C["dim"], None, False)
    rows.append(list(axis))
    return rows


# ---------------------------------------------------------------- screens

VIS = "▁ ▂ ▃ ▅ ▆ ▅ ▃ ▂ ▃ ▄ ▅ ▄ ▃ ▂ ▁ ▂ ▃ ▂ ▁ ▁ ▂ ▃ ▄ ▃ ▂ ▁ ▁ ▂ ▁ ▁ ▁ ▂ ▁ ▁ ▁ ▁ ▁ ▁ ▁ ▁ ▁ ▁ ▁ ▁ ▁"


def vol(p, w=26):
    n = round(p / 100 * w)
    return T("━" * n, "blue") + T("─" * (w - n), "faint")


def out_row(name, pct, preset, default, sel, muted_eq=False):
    mark = T(" ▌", "blue") if sel else T("  ")
    dot = T(" ● ", "green") if default else T("   ")
    left = mark + dot + T(f"{name:<24}", "fg", sel) + vol(pct) + T(f"  {pct:>3}%", "dim")
    chip = T(preset, "dim" if muted_eq else "magenta")
    return spread(left, chip + T("  "), W - 2)


def sound_main(outputs, sel, toast=None, footer=None):
    g = Grid()
    g.put(0, 0, T(" Sound", "fg", True))
    lines = [out_row(n, p, pr, d, i == sel) for i, (n, p, pr, d) in enumerate(outputs)]
    place(g, 2, 0, box("Output", lines, W, len(lines) + 2, active=True, right=T("EQ", "magenta")))
    r = 2 + len(lines) + 2
    place(g, r, 0, box("Input", [T("   ● ", "green") + T(f"{'MacBook Air Microphone':<24}") + vol(75) + T("   75%", "dim")], W, 3))
    r += 3
    place(g, r, 0, box("Playing", [T("     Spotify               ") + T("Hey Jude · The Beatles", "dim")], W, 3))
    g.put(H - 3, 0, T(VIS, "cyan"))
    g.put(H - 1, 0, footer or keys([("←→", "volume"), ("m", "mute"), ("e", "next preset"), ("E", "EQ"), ("f", "song"), ("?", "more"), ("esc", "close")]))
    if toast:
        t = T(" ") + toast + T(" ")
        g.put(H - 5, W - len(t) - 2, [(ch, fg, C["sel"], b) for ch, fg, _, b in t])
    return g


def eq_screen(device, kind, sel, current, on=True, tuned=None, note=None, footer=None):
    g = Grid()
    g.put(0, 0, spread(T(" Sound", "fg", True) + T("  ›  ", "dim") + T("EQ", "magenta", True), T(device, "dim") + T(" "), W))
    names = LISTS[kind]
    side_w = 26
    lines = []
    for i, name in enumerate(names):
        m = T(" ▌", "blue") if i == sel else T("  ")
        dot = T(" ● ", "magenta") if name == current else T("   ")
        cells = pad(m + dot + T(name, "fg", i == sel), side_w - 2)
        if i == sel:
            cells = [(ch, fg, C["sel"], b) for ch, fg, _, b in cells]
        lines.append(cells)
    place(g, 2, 0, box("Presets", lines, side_w, H - 4, active=True))
    pre, filters = P[current]
    shown = list(filters)
    if tuned:
        shown = shown + [("LSC", 110, tuned, 0.7)]
    pw = W - side_w
    status = T("on", "green") if on else T("off", "red")
    body = curve(shown, pw - 4, 14, on=on)
    body = [T(" ") + row for row in body]
    pre_total = pre - (max(0, tuned) if tuned else 0)
    body.append(T("  ") + T(f"preamp {pre_total:+g} dB", "dim") + (T("  ·  ", "faint") + describe(current) if filters else T("  ") + describe(current)))
    if note:
        body.append(T("  ") + note)
    title = current + (f" · bass {tuned:+g} dB" if tuned else "") + ("" if on else " (off)")
    place(g, 2, side_w, box(title, body, pw, H - 4, right=status))
    g.put(H - 1, 0, footer or keys([("↑↓", "preset"), ("b", "on/off"), ("←→", "bass"), ("r", "reset"), ("esc", "back")]))
    return g


def main():
    outputs = [("MacBook Air Speakers", 62, "Speakers +", True), ("EarFun Air Pro 4", 40, "Flat", False)]
    states = []

    def add(key, name, cap, g):
        states.append(dict(key=key, name=name, cap=cap, screen=g.html()))
        open(f"{key}.txt", "w").write(g.text() + "\n")

    add("main", "Sound", "The current preset sits next to each output. <kbd>e</kbd> steps to that output's next preset, so you hear it change right away. <kbd>E</kbd> opens the EQ.",
        sound_main(outputs, 0))
    add("cycle", "Press e", "One press of <kbd>e</kbd> on the speakers: <em>Speakers +</em> becomes <em>Speakers ++</em>, and a note says so for a second.",
        sound_main([("MacBook Air Speakers", 62, "Speakers ++", True), outputs[1]], 0,
                   toast=T("EQ ", "dim") + T("Speakers ++", "magenta", True) + T("  ·  e for next", "dim")))
    add("speakers", "EQ: speakers", "<kbd>↑</kbd> <kbd>↓</kbd> apply each preset live as you move, so you can compare by ear. The curve is what the EQ does; the faint bars behind it are what is playing right now.",
        eq_screen("MacBook Air Speakers", "speakers", 0, "Speakers +"))
    add("off", "Off (compare)", "<kbd>b</kbd> turns the EQ off and on for an A/B comparison. Off means telmo lets go of the audio completely, so nothing can get stuck.",
        eq_screen("MacBook Air Speakers", "speakers", 0, "Speakers +", on=False,
                  footer=keys([("b", "turn back on"), ("↑↓", "preset"), ("esc", "back")])))
    add("tune", "Nudge the bass", "<kbd>←</kbd> <kbd>→</kbd> nudge the bass in 1 dB steps on top of the preset. The change is remembered for this output; <kbd>r</kbd> goes back to the plain preset.",
        eq_screen("MacBook Air Speakers", "speakers", 0, "Speakers +", tuned=2,
                  footer=keys([("←→", "bass +2 dB"), ("r", "reset"), ("↑↓", "preset"), ("b", "on/off"), ("esc", "back")])))
    add("earfun", "EarFun Air Pro 4", "Your EarFun EQ (+8 / +7 / +5 at 31, 63, 125 Hz) can live on the earbuds or here. Here it also works with any app on the Mac, but the EarFun app must be set to flat, or the boost doubles.",
        eq_screen("EarFun Air Pro 4", "earfun", 1, "Your EarFun EQ",
                  note=T("Set the EarFun app's EQ to flat while this is on.", "yellow")))
    add("zerored", "Zero:RED", "With the Bass+ adapter it already sounds right, so the default adds only a little sub-bass. <em>Sub focus</em> lifts the deepest bass without touching 100–250 Hz.",
        eq_screen("External Headphones", "zerored", 1, "+ Sub-bass"))
    add("auto", "Auto switch", "Each output keeps its own preset. When the EarFuns connect, sound moves to them and their preset comes with them. Nothing to press.",
        sound_main([("MacBook Air Speakers", 62, "Speakers +", False), ("EarFun Air Pro 4", 40, "Your EarFun EQ", True)], 1,
                   toast=T("EarFun Air Pro 4 ", "fg") + T("·  EQ ", "dim") + T("Your EarFun EQ", "magenta", True)))
    json.dump(states, open("eq_states.json", "w"), ensure_ascii=False)


if __name__ == "__main__":
    main()
