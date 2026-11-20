#!/usr/bin/env python3
"""Generate the HARYX brand assets (SVG sources) into assets/brand/.

The mark ("Wing H", see docs/brand/logo-refinement.md) is two point-symmetric halves: Electric Cyan above, the
same shape rotated 180 degrees in Soft White below. Each half is a V. One arm rises into a square-cut vertical
stem, and the other sweeps up into a pointed wing tail that ends on the stem's top line. The four ends form the
legs of an H, and the interlocking halves form a fluid X.

Run: python3 scripts/brand/generate.py   (PNG and ICO exports: scripts/brand/export.sh)
"""
from pathlib import Path

OUT = Path(__file__).resolve().parents[2] / "assets" / "brand"

NAVY = "#101827"   # Midnight Navy
CYAN = "#22D3EE"   # Electric Cyan
SOFT = "#F1F5F9"   # Soft White
MONO = "currentColor"

# ---------- mark ----------
# Upper half in construction units, centred on (0, 0):
#   stem      x -16 .. -9.1, square cut on the top line y = -19
#   left arm  straight from the stem down to the V vertex (-4.5, 2.5)
#   tail      upper edge: one S-curve from the inner notch (-3.8, -5.4) to the tip (16, -19);
#             lower edge: one curve from the tip back to the V vertex
HALF = ("M-16 -19 L-9.1 -19 L-9.1 -11.7 L-3.8 -5.4 "
        "C3 -10.5 11.5 -12.8 16 -19 "
        "C14 -12.5 7 -6.2 -4.5 2.5 "
        "L-16 -11.7 Z")
HALF_SX, HALF_SY = 1.05, 1.2               # construction-to-mark stretch
MARK_W, MARK_H = 32 * HALF_SX, 38 * HALF_SY  # mark bounding box in mark units (33.6 x 45.6)


def g(v: float) -> str:
    return f"{v:.3f}".rstrip("0").rstrip(".")


def mark(top: str, bottom: str, height: float, cx: float, cy: float, bolden: float = 0.0) -> str:
    """The mark scaled to `height`, centred on (cx, cy). `bolden` adds an outline (construction units) that
    blunts the tips for very small sizes."""
    k = height / MARK_H

    def half(color: str, turn: str) -> str:
        outline = (f' stroke="{color}" stroke-width="{g(bolden)}" stroke-linejoin="round"' if bolden else "")
        return f'<path d="{HALF}" fill="{color}"{outline} transform="{turn}scale({g(HALF_SX)} {g(HALF_SY)})"/>'

    return (f'<g transform="translate({g(cx)} {g(cy)}) scale({g(k)})">'
            + half(bottom, "rotate(180) ") + half(top, "") + "</g>")


# ---------- wordmark ----------
# Geometric monoline capitals on a 40-unit cap height: stroke 7, square-cut (butt) ends that sit on the cap and
# base lines, bevelled apexes. The cuts echo the mark's square stems.
CAP = 40
LETTER_STROKE = 7
TRACKING = 8
LETTERS = {
    "H": (30, ["M3.5 0 V40", "M26.5 0 V40", "M3.5 20 H26.5"]),
    "A": (33, ["M3.5 40 L16.5 3 L29.5 40", "M9.2 27 H23.8"]),
    "R": (29, ["M3.5 40 V3.5 H15 A9.5 9.5 0 0 1 15 22.5 H3.5", "M14.5 22.5 L26 40"]),
    "Y": (31, ["M3.5 0 L15.5 18 L27.5 0", "M15.5 18 V40"]),
    "X": (31, ["M3.5 0 L27.5 40", "M27.5 0 L3.5 40"]),
}


def wordmark(color: str) -> tuple[str, float]:
    """HARYX on the 40-unit cap grid. Returns (svg body, advance width)."""
    parts, x = [], 0.0
    for ch in "HARYX":
        width, paths = LETTERS[ch]
        strokes = "".join(
            f'<path d="{d}" fill="none" stroke="{color}" stroke-width="{LETTER_STROKE}" '
            f'stroke-linecap="butt" stroke-linejoin="bevel"/>' for d in paths)
        parts.append(f'<g transform="translate({g(x)} 0)">{strokes}</g>')
        x += width + TRACKING
    return "".join(parts), x - TRACKING


# ---------- documents ----------

def svg(view_box: str, body: str, title: str = "HARYX") -> str:
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view_box}" role="img" aria-label="{title}">'
            f'<title>{title}</title>{body}</svg>\n')


def lockup(top: str, bottom: str, word: str) -> str:
    """Symbol + HARYX: mark 56 high, cap height 26, optically centred, gap of one stem width."""
    height, cap_h = 56.0, 26.0
    mark_w = MARK_W * height / MARK_H
    body_word, word_w = wordmark(word)
    scale = cap_h / CAP
    left = 4 + mark_w + 12
    total = left + word_w * scale + 4
    body = (mark(top, bottom, height, 4 + mark_w / 2, 32)
            + f'<g transform="translate({g(left)} {g(32 - cap_h / 2)}) scale({g(scale)})">{body_word}</g>')
    return svg(f"0 0 {g(total)} 64", body)


def app_icon() -> str:
    """macOS app icon: navy rounded square on Apple's 824-on-1024 grid, mark at ~60% of the tile height."""
    return svg("0 0 1024 1024",
               f'<rect x="100" y="100" width="824" height="824" rx="185" fill="{NAVY}"/>'
               + mark(CYAN, SOFT, 500, 512, 512))


def windows_icon() -> str:
    """Windows / generic app icon: full-bleed tile with a smaller corner radius."""
    return svg("0 0 256 256", f'<rect width="256" height="256" rx="40" fill="{NAVY}"/>' + mark(CYAN, SOFT, 168, 128, 128))


def favicon() -> str:
    """Full-bleed tile, mark enlarged and slightly boldened so the tips survive 16 x 16."""
    return svg("0 0 64 64", f'<rect width="64" height="64" rx="12" fill="{NAVY}"/>'
               + mark(CYAN, SOFT, 48, 32, 32, bolden=1.2))


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    word_body, word_w = wordmark(MONO)
    files = {
        # Symbol only (on a 64 grid).
        "haryx-mark.svg": svg("0 0 64 64", mark(CYAN, SOFT, 56, 32, 32)),
        "haryx-mark-light.svg": svg("0 0 64 64", mark(CYAN, NAVY, 56, 32, 32)),
        "haryx-mark-mono.svg": svg("0 0 64 64", mark(MONO, MONO, 56, 32, 32)),
        # Symbol + HARYX.
        "haryx-lockup.svg": lockup(CYAN, SOFT, SOFT),
        "haryx-lockup-light.svg": lockup(CYAN, NAVY, NAVY),
        "haryx-lockup-mono.svg": lockup(MONO, MONO, MONO),
        "haryx-wordmark.svg": svg(f"-2 -2 {g(word_w + 4)} {CAP + 4}", word_body),
        # Icons.
        "haryx-app-icon.svg": app_icon(),
        "haryx-app-icon-windows.svg": windows_icon(),
        "favicon.svg": favicon(),
    }
    for name, content in files.items():
        (OUT / name).write_text(content)
        print(f"wrote {OUT / name}")


if __name__ == "__main__":
    main()
