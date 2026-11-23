---
name: HaryX
description: A unified control center for AI coding accounts, usage and sessions, native to the macOS menu bar.
colors:
  electric-cyan: "#22D3EE"
  electric-cyan-light: "#0891B2"
  cyan-ink: "#0E7490"
  cyan-text-light: "#155E75"
  midnight-navy: "#101827"
  soft-white: "#F1F5F9"
  pace-amber: "#FF9F0A"
  limit-red: "#FF453A"
typography:
  display:
    fontFamily: "Space Grotesk, SF Pro Display, system-ui"
    fontSize: "44pt"
    fontWeight: 600
    lineHeight: 1
    fontFeature: "tnum"
  display-card:
    fontFamily: "Space Grotesk, SF Pro Display, system-ui"
    fontSize: "28pt"
    fontWeight: 600
    lineHeight: 1
    fontFeature: "tnum"
  clock:
    fontFamily: "Space Grotesk, SF Pro Display, system-ui"
    fontSize: "18pt"
    fontWeight: 500
    fontFeature: "tnum"
  wordmark:
    fontFamily: "Space Grotesk, SF Pro Display, system-ui"
    fontSize: "13pt"
    fontWeight: 700
    letterSpacing: "0.11em"
  headline:
    fontFamily: "SF Pro Text, system-ui"
    fontSize: "13pt"
    fontWeight: 600
  body:
    fontFamily: "SF Pro Text, system-ui"
    fontSize: "12pt"
    fontWeight: 400
    fontFeature: "tnum"
  caption:
    fontFamily: "SF Pro Text, system-ui"
    fontSize: "10pt"
    fontWeight: 400
  badge:
    fontFamily: "SF Pro Text, system-ui"
    fontSize: "9pt"
    fontWeight: 700
rounded:
  staff: "1.5px"
  banner: "10px"
  card: "16px"
  pill: "999px"
spacing:
  hair: "2px"
  row: "5px"
  tight: "8px"
  rule: "10px"
  panel: "14px"
  grid: "16px"
  pane: "20px"
components:
  button-primary:
    backgroundColor: "{colors.cyan-ink}"
    textColor: "#FFFFFF"
    rounded: "{rounded.pill}"
    typography: "{typography.body}"
  button-glass:
    textColor: "{colors.soft-white}"
    rounded: "{rounded.pill}"
  button-sign-in:
    backgroundColor: "{colors.pace-amber}"
    textColor: "#FFFFFF"
    rounded: "{rounded.pill}"
  badge-active:
    textColor: "{colors.electric-cyan}"
    typography: "{typography.badge}"
    rounded: "{rounded.pill}"
    padding: "2px 6px"
  tide-staff:
    backgroundColor: "{colors.electric-cyan}"
    rounded: "{rounded.staff}"
    height: "6px"
  tide-staff-card:
    backgroundColor: "{colors.electric-cyan}"
    rounded: "{rounded.staff}"
    height: "5px"
  account-card:
    rounded: "{rounded.card}"
    padding: "14px"
  account-card-selected:
    rounded: "{rounded.card}"
    padding: "14px"
  almanac-row:
    typography: "{typography.body}"
    height: "30px"
    padding: "5px 0"
  brand-tile:
    backgroundColor: "{colors.midnight-navy}"
---

# Design System: HaryX

## Overview

**Creative North Star: "The Tide Table"**

Each account's 5-hour window is read the way a harbour master reads a tide table: how high the water is now, where the prediction says it should be, and when the next low water comes. The system is a native macOS one. The ground is the system's own Liquid Glass and Light/Dark appearance; HaryX adds very little on top of it: one cyan for what is live, an amber and a red for pace and limits, a geometric display face for the numbers that matter, and hairline rules in place of boxes.

Density is that of an almanac rather than a dashboard of widgets. The menu bar panel (380pt wide) puts the active account first with its 5-hour percentage set large, a flat tide staff under it, and the rest of the accounts below a hairline as a fixed-column table in switch order. Type weight and size carry the hierarchy; chrome is removed wherever a rule or a column can do the job.

The dashboard window is a standard sidebar/detail split. Accounts are flat filled cards, active account first and then in switch order, that repeat the panel's tide reading at a smaller scale (percentage, staff with pace tick, time-left rule, pace delta, week), so the reading looks the same wherever it appears. Glass belongs to the controls and navigation that sit over this content, never to the content itself.

**Key Characteristics:**
- System Liquid Glass for controls, navigation and floating notices; content sits on flat system fills. System Light/Dark throughout.
- Electric Cyan marks the active account, live state and on-pace usage. Amber means ahead of pace or past 85%; red only at the limit.
- Space Grotesk only for the wordmark, large percentages and reset clocks; SF Pro everywhere else.
- Tabular figures in every number that sits in a column or updates in place.
- 1pt hairline rules divide sections; flat fills, no gradients, no authored shadows.

## Colors

One live hue against the system's neutrals, with the system's warning colours carrying pace and limits.

### Primary
- **Electric Cyan** (dark appearance; Electric Cyan Light in light appearance): the active account, live states (running dot, the auto-switch "on" label), the on-pace fill of the tide staff, and the time-left rule. It is a dynamic colour: the lighter swatch in dark mode, the deeper one on white so it keeps contrast.
- **Cyan Ink**: the app's AccentColor asset, so it is the fill of prominent glass buttons (white text, 5.4:1) and, at 18% opacity, the fill of a selected dashboard card.
- **Cyan Text (light)** `#155E75`: cyan used as text in the light appearance; holds 5.3:1 down to light glass at about #DCDCDC, where Cyan Ink falls to 3.9:1. In dark mode cyan text returns to Electric Cyan (9.2:1 on dark glass).

### Secondary
- **Midnight Navy**: the tile behind the Wing H mark, and nothing else in the shipped build.
- **Soft White**: the lower half of the Wing H mark on its navy tile.

### Tertiary
- **Pace Amber** (system orange): usage ahead of the predicted pace, usage at or past 85%, statuses that need attention (re-login needed), and the tint of the prominent "Sign In Again" button.
- **Limit Red** (system red): a window at 100%, and error banners and cards. Nothing else.

### Neutral
- **System label tiers** (primary / secondary / tertiary / quaternary): all text and the empty track of the tide staff. HaryX defines no neutral greys of its own; the system's hierarchical styles adapt to glass and appearance.
- **System separator**: every hairline rule.

### Named Rules
**The Live Cyan Rule.** Cyan means live or on pace: the active account, a running session, an enabled automation, a fill that is on pace. Do not use it for decoration or for a heading. Small live text on glass is carried by a cyan dot beside secondary text, not by cyan text.

**The Three-Step Fill Rule.** A usage reading is cyan while on pace, amber when ahead of pace or at 85% or more, red at 100%. Ahead of pace means 5 or more points over the expected pace when a pace is known, otherwise the CLI's own verdict; drift under 5 points is on pace. In almanac columns a percentage is coloured only from 85% up; below that it stays primary text.

## Typography

**Display Font:** Space Grotesk (bundled; falls back to the system face)
**Body Font:** SF Pro (system)

**Character:** A geometric grotesk with open, slightly mechanical numerals for the figures you glance at, alongside the system face for everything you read. The pairing keeps the panel native while giving the readings an instrument look.

### Hierarchy
- **Display** (Space Grotesk 600, 44pt, tabular): the active account's 5-hour percentage in the menu bar panel. One per surface.
- **Display Card** (Space Grotesk 600, 28pt, tabular): the 5-hour percentage on each dashboard account card.
- **Clock** (Space Grotesk 500, 18pt, tabular): the reset time beside the display figure.
- **Wordmark** (Space Grotesk 700, 0.72 × the tile size, tracking 0.08 × the tile size, uppercase "HARYX"): the lockup only.
- **Headline** (SF Pro semibold, 13pt): account names.
- **Body / Callout** (SF Pro, 12pt, tabular figures in tables): almanac rows, notes.
- **Caption** (SF Pro, 10pt): reset countdowns, pace lines, plan and slot metadata, column headers (tertiary).
- **Badge** (SF Pro bold, 9pt, uppercase): plan and state badges (ACTIVE, TEAM, PRO, OFF).

### Named Rules
**The Display-Only Rule.** Space Grotesk appears only in the wordmark, the large percentages and the reset clocks. Labels, buttons, headings and tables stay in SF Pro.

**The Tabular Figures Rule.** Every number that sits in a column or refreshes in place uses tabular (monospaced) digits, so the columns do not move when the values change.

## Layout

The menu bar panel is 380pt wide with 14pt padding on all sides, stacked vertically in this order: header (lockup, auto-switch state, refresh / add / more), a hairline, the active-account tide block, a hairline, the "Next in line" almanac, the last auto-switch event, an optional banner, and the footer actions. Hairlines get 10pt of space above and below; the event and banner lines sit 8pt below the content above them; the footer actions sit 12pt below. The footer is three controls in an 8pt row: Switch (the one prominent action, filling the remaining width), Dashboard (icon-only glass split control, dashboard gauge symbol) and Terminal (glass split button). The two glass controls share one form, so they read as a pair. Switch strategies live in the header's more menu, not in the footer.

The almanac is a grid with fixed trailing-aligned columns (5h, 7d, Resets), 10pt column spacing, 30pt rows (5pt vertical padding), scrolling past a 300pt cap. Row actions ("Use") appear only on hover.

The dashboard is a sidebar (190–210pt) / detail split with a 300–420pt inspector. Account cards sit in an adaptive grid (minimum 250pt columns, 16pt gaps, 20pt pane inset), with 14pt internal padding. Cards run active account first, then switch order, matching the panel. Every card is laid out the same way whether usage is known or not, so the rows of cards stay the same height.

## Elevation & Depth

Two layers. The functional layer (the menu bar panel itself, buttons, toolbar, sidebar, floating banners and the panel's error notice) is the system's Liquid Glass; the panel groups its glass in a `GlassEffectContainer` (12pt spacing). The content layer (account cards) is flat: a quaternary system fill, or the accent at 18% when selected, with no glass, no stroke and no shadow. HaryX authors no shadows anywhere. A glass notice may take a state tint: cyan or red at 12% for a banner, red at 10% for an error notice.

### Named Rules
**The Glass Is Chrome Rule.** Glass is for controls, navigation and floating notices. Content (account cards and readings) sits on flat system fills, and selection is a fill change, not a ring or a glass tint. Never author shadows or gradients.

## Shapes

Two shape families. Readings are almost square: the tide staff is a flat rectangle with a 1.5pt radius, the pace tick and time-left rule are 1pt lines. Containers are soft: dashboard cards and the panel's error notice at 16pt, banners at 10pt, buttons and badges as capsules. The Wing H tile is rounded at 24% of its size. Sections are divided by 1pt rules; containers carry no strokes at all, selected or not.

## Components

### Buttons
- **Shape:** system capsule (glass button styles).
- **Primary:** prominent glass in the accent (Cyan Ink fill, white text), for the one main action on a surface: "Switch to <alias or email local part>" in the panel footer and "Switch" on a card. Large control size in the panel footer, small on cards.
- **Glass:** plain glass for secondary actions next to it: in the panel footer, the Dashboard control (an icon-only glass menu whose primary action opens the dashboard, symbol `gauge.with.dots.needle.50percent`, labelled "Open the dashboard" for assistive technology, menu indicator hidden) and the Terminal split button; on cards, the terminal split button. Icon-only controls always carry an accessibility label and a help tooltip.
- **Sign In Again:** prominent glass tinted amber when a re-login is needed; plain glass when the login is merely due to expire. While a sign-in is pending it becomes a mini spinner with "Waiting for sign-in…".
- **Borderless:** header icons (SF Symbols in secondary) and the hover-revealed "Use" in almanac rows.

### Chips
- **Plan badge:** 9pt bold uppercase text in the tint colour on the same tint at 18% opacity, capsule, 6pt × 2pt padding. ACTIVE uses cyan text; plan names use secondary; OFF uses grey.
- **Running indicator:** a 6pt Electric Cyan dot beside "N running" in secondary text: in the panel, 4pt from the surrounding caption; in dashboard card badges, 3pt from 9pt semibold "running" / "N running". The count is never cyan text.

### Cards / Containers
- **Corner Style:** 16pt.
- **Background:** flat quaternary system fill; the accent at 18% opacity when selected.
- **Shadow Strategy:** none (see Elevation & Depth).
- **Border:** none, in every state.
- **Internal Padding:** 14pt; 10pt between internal rows. The whole rounded card is the hit target.
- **Content:** mascot and name block, then the card tide reading (28pt figure, 5pt staff, time-left rule, pace delta in caption, week line), the status or wake note, and the action row.

### Navigation
The dashboard sidebar is a standard source list grouped by domain ("AI", then "HaryX" for app settings), SF Symbol labels, with the brand lockup (20pt tile) inset above the list. Selection uses the system highlight.

### Tide Staff (signature)
The usage reading, and the only usage bar in the app. A flat bar, 6pt tall in the panel and 5pt on cards and in the inspector, 1.5pt corner radius: quaternary track, fill coloured by the Three-Step Fill Rule, and a 1pt primary-colour tick at the expected pace that overhangs the bar by 3pt above and below (it is not clipped by the bar's corners). The 5-hour window always has a pace: the CLI's expected pace when it sends one, otherwise linear over the window from the reset time. Pace text uses a 5-point threshold: at 5 or more points off the pace the line names the delta ("N pts ahead of pace" in amber, "N pts under pace" in secondary); under 5 points the panel says "on pace" in secondary, and only when a pace reading exists. Cards show only the off-pace text and stay silent when on pace. Under the 5-hour staff, a 1pt cyan line at 55% opacity shows the time left in the window as a length: full width means the whole window remains.

### Usage Bar
The labelled form of the Tide Staff, used in the account inspector: a sentence-case label ("5-hour", "Week") in medium secondary caption with the reset text in tertiary tabular caption on the same line, then a 5pt Tide Staff with the percentage in medium tabular callout in a 40pt trailing column.

### Almanac Row (signature)
One account per row in the "Next in line" table: an 18pt mascot, the name (middle-truncated) with its plan in tertiary caption, then 5h %, 7d % and reset clock in fixed trailing columns, all in tabular 12pt. A disabled account is shown at 50% opacity. Hovering adds a quaternary wash at 50% and shows "Use".

### Brand Lockup
Wing H mark on its Midnight Navy tile (upper half Electric Cyan, lower half Soft White), followed by "HARYX" in the wordmark style. In the status bar the mark is a 16pt template image, with a 4pt dot added while sessions are running.

## Do's and Don'ts

### Do:
- **Do** put the active account's 5-hour percentage first, as the one Display figure on the surface, with the reset clock right-aligned beside it.
- **Do** draw every usage reading as a Tide Staff (flat fill, 1.5pt radius, pace tick) coloured cyan / amber / red by the Three-Step Fill Rule.
- **Do** use tabular figures for every percentage, clock and countdown.
- **Do** divide panel sections with 1pt system-separator rules and 10pt of space on each side.
- **Do** use the system label tiers for all neutral text, keep content on flat system fills, and show selection as the accent fill at 18%.

### Don't:
- **Don't** set labels, buttons, headings or table text in Space Grotesk; it is reserved for the wordmark, large figures and clocks.
- **Don't** use cyan for anything that is not live, active or on pace, and don't use red before a window is at its limit.
- **Don't** put content on glass or add gradients and authored shadows; cards are flat system fills and glass is for controls, navigation and floating notices.
- **Don't** put borders or rings on containers, including for selection; selection is the accent fill at 18%.
- **Don't** draw a usage reading as a capsule progress bar or label it in uppercase; use the Tide Staff and sentence case.
- **Don't** use Midnight Navy as a surface or text colour; it exists as the mark's tile.
