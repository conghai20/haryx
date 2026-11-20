# HARYX: Logo Refinement Brief

Status: **final. "Wing H" is the HARYX logo** (section 8). The source of truth is `scripts/brand/generate.py`,
and the exports are in `assets/brand/`.

Goal: refine and improve the existing HARYX mark without reinventing it. Keep what makes it recognizable and
improve its geometry, balance, clarity and craftsmanship.

## 1. Concept to preserve

The current mark (`assets/brand/haryx-app-icon.svg`) is the foundation:

- A deep midnight navy rounded square.
- A bold, flowing cyan diagonal stroke (top-left to bottom-right) that forms an abstract X.
- Two small white organic fragments at opposite corners (top-right, bottom-left) that make up the X's
  second stroke.
- Rounded edges and a fluid, asymmetric composition.

Do not replace this with an unrelated symbol.

### Known weaknesses of the current mark

- The white fragments end in angled butt cuts. Next to the round-capped cyan stroke they look clipped and
  mechanical rather than designed.
- The fragments follow a different curvature from the cyan stroke, so the X reads as two unrelated gestures.
- The gap between the fragments and the cyan stroke is too wide and uneven, so the under-stroke reads as broken.
- The mark nearly fills the tile, which leaves the composition little breathing room at app-icon sizes.

## 2. Design philosophy

- **Minimalism:** remove anything that does not carry the idea.
- **Swiss graphic design:** strong proportions, intentional geometry, visual balance.
- **Contemporary technology branding:** distinctive and sophisticated, at the level of mature independent
  software brands (Linear, Vercel, Raycast) without imitating their logos.
- **Abstract organic geometry:** smooth curves built with precise construction.

## 3. Refinement targets

**Geometry and proportions**
- One stroke weight across the whole symbol, with controlled, constructed curvature.
- The fragments must feel integrated with the cyan stroke, not decorative additions.
- Balanced but not perfectly symmetrical.

**Shape and negative space**
- An intentional, consistent channel of negative space between the cyan stroke and each fragment.
- A stronger visual relationship between all three elements.
- Keep the flowing movement. No unnecessary intersections or clutter.

**Color:** flat and solid only. No gradients, glows, shadows or metallic effects.

| Token          | Hex       | Use                                   |
| -------------- | --------- | ------------------------------------- |
| Midnight Navy  | `#101827` | Tile background, dark surfaces        |
| Electric Cyan  | `#22D3EE` | Primary stroke                        |
| Soft White     | `#F1F5F9` | Fragments, light backgrounds          |

This palette replaces the earlier Midnight `#111827` / Silver `#E5E7EB` / White values in `PRODUCT.md`.

## 4. Symbolism (expressed through geometry only)

- **Aquarius:** fluidity, independence, unconventional thinking.
- **X:** experimentation, switching, connectivity.
- **Flow:** continuous movement between AI accounts, sessions and workflows.

Not allowed: zodiac illustrations, water droplets, robots, brains, circuit-board patterns.

## 5. Practical requirements

The mark must work as a desktop app icon, macOS Dock icon, Windows app icon, website favicon, GitHub repository
branding, and in monochrome documentation and terminal contexts. It must stay recognizable at 16, 32, 64 and
256 px.

Required versions:
- Primary (dark background)
- Light background
- Monochrome
- Symbol only
- Symbol + HARYX wordmark lockup (modern geometric sans: confident, clean, slightly futuristic, not sci-fi)

## 6. Deliverables (after a variant is chosen)

- Refined primary logo
- Side-by-side comparison with the original
- Compact construction and proportion guide
- Color specifications
- Typography recommendations
- App icon preview
- Light and dark variations

## 7. Exploration history

Five rounds were explored. All variants except the chosen direction have been removed.

1. **Round 1:** refinements of the original mark (Flow, Channel, Axis) with round caps.
2. **Round 2:** the same refinements with square-cut ends. Owner preference: square-cut ends, fluid X kept.
3. **Round 3:** the "wing" construction from the owner's style reference
   (https://www.behance.net/gallery/182897005/X-Logos). The X is two point-symmetric halves, each a V with one
   straight square-cut arm and one sweeping tail.
4. **Round 4:** an H built into the fluid X. Every arm ends in a vertical stem, and the stems form the H's legs.
5. **Round 5:** H stems on the straight arms, with pointed wing tails on the other arms. The owner chose **M**
   (rising tail) and asked for two fixes: more natural curves, and equal height on the left and right of each half.

## 8. Chosen direction: M "Wing H"

Source: `scripts/brand/generate.py` (SVG) and `scripts/brand/export.sh` (PNG and ICO).

**Construction.** Two halves are point-symmetric about the centre. The upper half is Electric Cyan; the lower
half is the same shape rotated 180° in Soft White. Each half is a V:

- **Stem:** the straight arm rises into a vertical stem with a square cut on the top line.
- **Wing tail:** the other arm sweeps up into a pointed tail. Its upper edge is one continuous S-curve from the
  inner notch to the tip, and its lower edge is one continuous curve from the tip back to the V vertex. There are
  no straight-to-curve kinks.
- **Equal height:** the tail tip lands exactly on the stem's top line, so both sides of each half are the same
  height. It also lands on the outer edge of the opposite half's stem, so the whole mark sits in a symmetric box.
- **Gap:** the two halves interlock across a narrow diagonal gap, which acts as the H's crossbar.

**Reading.** The four ends sit on two vertical lines: square stems at top-left and bottom-right, pointed tips at
top-right and bottom-left. Together they form the legs of an **H** (Haryo, HARYX). The interlocking V and
inverted V form the fluid **X**.

**Symbolism.** Two mirrored, flowing halves stand for Aquarius (fluidity, independence). The crossing stands for
X (switching, connectivity). The continuous sweep from stem to tail stands for flow between accounts, sessions
and workflows.

**Small sizes.** The pointed tips thin out at 16 px, so `favicon.svg` enlarges the mark and adds a thin outline
in each half's own colour. This blunts the tips slightly, and the favicon then still reads as the symbol at 16 px.

**Originality.** The wing construction comes from a style reference made by another designer. M differs from it
in its square-cut H stems, its tail geometry (rising to the stem line rather than sweeping level), its
proportions and its palette.

Decision: **final** (owner approved).

## 9. Asset suite

| File | Use |
| ---- | --- |
| `haryx-mark.svg` | Symbol on dark backgrounds (cyan and soft white) |
| `haryx-mark-light.svg` | Symbol on light backgrounds (cyan and navy) |
| `haryx-mark-mono.svg` | Single colour (`currentColor`) for docs, terminal and print |
| `haryx-lockup.svg`, `-light`, `-mono` | Symbol + HARYX wordmark: dark, light, mono |
| `haryx-wordmark.svg` | HARYX wordmark alone (`currentColor`) |
| `haryx-app-icon.svg` → `png/haryx-app-icon-1024.png` | macOS app and Dock icon (Apple 824-on-1024 grid); used by `apps/macos/scripts/build.sh` |
| `haryx-app-icon-windows.svg` → `haryx.ico` | Windows app icon (16–256 px) |
| `favicon.svg` → `favicon.ico`, `png/favicon-*.png` | Website favicon and touch icon (180 px) |
| `png/haryx-mark-*.png`, `png/haryx-lockup*.png` | Preview renders, e.g. GitHub README and social |

**Wordmark.** Custom geometric monoline capitals on a 40-unit cap height, with a stroke of 7. Ends are square-cut
to echo the mark's stems, and apexes are bevelled. In the lockup, the mark is 56 high and the cap height is 26.
For running text and UI, set type in Space Grotesk, with Geist as the alternative.

**Colour.**

| Token | Hex | RGB |
| ----- | --- | --- |
| Midnight Navy | `#101827` | 16, 24, 39 |
| Electric Cyan | `#22D3EE` | 34, 211, 238 |
| Soft White | `#F1F5F9` | 241, 245, 249 |

Use flat colour only: no gradients, glows or shadows. On light backgrounds, Navy replaces Soft White as the
second colour.
