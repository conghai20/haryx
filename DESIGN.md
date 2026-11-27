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
  web-ink-2: "#3D4A5E"
  web-ink-3: "#5B6A80"
  web-ink-2-dark: "#B4C0D0"
  web-ink-3-dark: "#8A98AD"
  web-amber-text-light: "#C26A00"
  web-rule: "rgb(16 24 39 / 0.14)"
  web-rule-strong: "rgb(16 24 39 / 0.32)"
  web-rule-dark: "rgb(241 245 249 / 0.12)"
  web-rule-strong-dark: "rgb(241 245 249 / 0.3)"
  web-track: "rgb(16 24 39 / 0.08)"
  web-track-dark: "rgb(241 245 249 / 0.1)"
  web-live-wash: "rgb(8 145 178 / 0.12)"
  web-live-wash-dark: "rgb(34 211 238 / 0.14)"
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
  web-display:
    fontFamily: "Space Grotesk Variable, Space Grotesk, system-ui, sans-serif"
    fontSize: "clamp(3rem, min(7.4vw, 10.5vh), 6rem)"
    fontWeight: 500
    lineHeight: 0.96
    letterSpacing: "-0.04em"
  web-close:
    fontFamily: "Space Grotesk Variable, Space Grotesk, system-ui, sans-serif"
    fontSize: "clamp(2.75rem, 6vw, 5rem)"
    fontWeight: 500
    lineHeight: 1
    letterSpacing: "-0.035em"
  web-headline:
    fontFamily: "Space Grotesk Variable, Space Grotesk, system-ui, sans-serif"
    fontSize: "clamp(2.25rem, 5vw, 4rem)"
    fontWeight: 500
    lineHeight: 1.02
    letterSpacing: "-0.03em"
  web-figure:
    fontFamily: "Space Grotesk Variable, Space Grotesk, system-ui, sans-serif"
    fontSize: "clamp(1.75rem, 3vw, 2.5rem)"
    fontWeight: 500
    lineHeight: 1
    letterSpacing: "-0.02em"
    fontFeature: "tnum"
  web-title:
    fontFamily: "Space Grotesk Variable, Space Grotesk, system-ui, sans-serif"
    fontSize: "1.375rem"
    fontWeight: 500
    letterSpacing: "-0.01em"
  web-statement:
    fontFamily: "Geist Variable, Geist, system-ui, sans-serif"
    fontSize: "clamp(1.375rem, 2.3vw, 2rem)"
    fontWeight: 450
    lineHeight: 1.2
    letterSpacing: "-0.015em"
  web-lede:
    fontFamily: "Geist Variable, Geist, system-ui, sans-serif"
    fontSize: "clamp(1.0625rem, 1.4vw, 1.25rem)"
    fontWeight: 400
    lineHeight: 1.6
  web-body:
    fontFamily: "Geist Variable, Geist, system-ui, sans-serif"
    fontSize: "1.0625rem"
    fontWeight: 400
    lineHeight: 1.6
    fontFeature: "tnum"
  web-small:
    fontFamily: "Geist Variable, Geist, system-ui, sans-serif"
    fontSize: "0.875rem"
    fontWeight: 400
  web-command:
    fontFamily: "Geist Mono Variable, Geist Mono, ui-monospace, monospace"
    fontSize: "0.875rem"
    fontWeight: 400
    lineHeight: 1.5
rounded:
  staff: "1.5px"
  banner: "10px"
  card: "16px"
  pill: "999px"
  web-field: "2px"
spacing:
  hair: "2px"
  row: "5px"
  tight: "8px"
  rule: "10px"
  panel: "14px"
  grid: "16px"
  pane: "20px"
  web-gutter: "clamp(16px, 4.5vw, 72px)"
  web-column-gap: "clamp(16px, 2vw, 32px)"
  web-section: "clamp(96px, 14vw, 200px)"
  web-max: "1440px"
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
  web-command-field:
    textColor: "{colors.midnight-navy}"
    typography: "{typography.web-command}"
    rounded: "{rounded.web-field}"
    padding: "14px 16px"
  web-command-field-dark:
    textColor: "{colors.soft-white}"
    typography: "{typography.web-command}"
    rounded: "{rounded.web-field}"
    padding: "14px 16px"
  web-copy-button:
    backgroundColor: "{colors.midnight-navy}"
    textColor: "{colors.soft-white}"
    rounded: "{rounded.web-field}"
    padding: "0 22px"
    height: "48px"
  web-copy-button-dark:
    backgroundColor: "{colors.soft-white}"
    textColor: "{colors.midnight-navy}"
    rounded: "{rounded.web-field}"
    padding: "0 22px"
    height: "48px"
  web-copy-button-hover-dark:
    backgroundColor: "{colors.electric-cyan}"
    textColor: "{colors.midnight-navy}"
  web-tide-staff:
    backgroundColor: "{colors.electric-cyan-light}"
    rounded: "{rounded.staff}"
    height: "6px"
  web-tide-staff-dark:
    backgroundColor: "{colors.electric-cyan}"
    rounded: "{rounded.staff}"
    height: "6px"
  web-badge-active:
    backgroundColor: "{colors.web-live-wash}"
    textColor: "{colors.cyan-ink}"
    rounded: "{rounded.pill}"
    padding: "2px 8px"
---

# Design System: HaryX

## Overview

**Creative North Star: "The Tide Table"**

Each account's 5-hour window is read the way a harbour master reads a tide table: how high the water is now, where the prediction says it should be, and when the next low water comes. The system is a native macOS one. The ground is the system's own Liquid Glass and Light/Dark appearance; HaryX adds very little on top of it: one cyan for what is live, an amber and a red for pace and limits, a geometric display face for the numbers that matter, and hairline rules in place of boxes.

Density is that of an almanac rather than a dashboard of widgets. The menu bar panel (380pt wide) puts the active account first with its 5-hour percentage set large, a flat tide staff under it, and the rest of the accounts below a hairline as a fixed-column table in switch order. Type weight and size carry the hierarchy; chrome is removed wherever a rule or a column can do the job.

The dashboard window is a standard sidebar/detail split. Accounts are flat filled cards, active account first and then in switch order, that repeat the panel's tide reading at a smaller scale (percentage, staff with pace tick, pace delta, week staff), so the reading looks the same wherever it appears. Glass belongs to the controls and navigation that sit over this content, never to the content itself.

**Key Characteristics:**
- System Liquid Glass for controls, navigation and floating notices; content sits on flat system fills. System Light/Dark throughout.
- Electric Cyan marks the active account, live state and on-pace usage. Amber means ahead of pace or past 85%; red only at the limit.
- Space Grotesk only for the wordmark, large percentages and reset clocks; SF Pro everywhere else.
- Tabular figures in every number that sits in a column or updates in place.
- 1pt hairline rules divide sections; flat fills, no gradients, no authored shadows.

## Colors

One live hue against the system's neutrals, with the system's warning colours carrying pace and limits.

### Primary
- **Electric Cyan** (dark appearance; Electric Cyan Light in light appearance): the active account, live states (running dot, the auto-switch "on" label), and the on-pace fill of the tide staff. It is a dynamic colour: the lighter swatch in dark mode, the deeper one on white so it keeps contrast.
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

The almanac is a grid with fixed trailing-aligned columns (5h, 7d, Resets), 10pt column spacing, 30pt rows (5pt vertical padding) inset 8pt inside the full-width hover fill, scrolling past a 300pt cap. Every row carries its "Use" action.

The dashboard is a sidebar (190–210pt) / detail split with a 300–420pt inspector. Account cards sit in an adaptive grid (minimum 250pt columns, 16pt gaps, 20pt pane inset), with 14pt internal padding. Cards run active account first, then switch order, matching the panel. Every card is laid out the same way whether usage is known or not, so the rows of cards stay the same height.

## Elevation & Depth

Two layers. The functional layer (the menu bar panel itself, buttons, toolbar, sidebar, floating banners and the panel's error notice) is the system's Liquid Glass; the panel groups its glass in a `GlassEffectContainer` (12pt spacing). The content layer (account cards) is flat: a quaternary system fill, or the accent at 18% when selected, with no glass, no stroke and no shadow. HaryX authors no shadows anywhere. A glass notice may take a state tint: cyan or red at 12% for a banner, red at 10% for an error notice.

### Named Rules
**The Glass Is Chrome Rule.** Glass is for controls, navigation and floating notices. Content (account cards and readings) sits on flat system fills, and selection is a fill change, not a ring or a glass tint. Never author shadows or gradients.

## Shapes

Two shape families. Readings are almost square: the tide staff is a flat rectangle with a 1.5pt radius, the pace tick is a 1pt line. Containers are soft: dashboard cards and the panel's error notice at 16pt, banners at 10pt, buttons and badges as capsules. The Wing H tile is rounded at 24% of its size. Sections are divided by 1pt rules; containers carry no strokes at all, selected or not.

## Components

### Buttons
- **Shape:** system capsule (glass button styles).
- **Primary:** prominent glass in the accent (Cyan Ink fill, white text), for the one main action on a surface: "Switch to <alias or email local part>" in the panel footer and "Switch" on a card. Large control size in the panel footer, small on cards.
- **Glass:** plain glass for secondary actions next to it: in the panel footer, the Dashboard control (an icon-only glass menu whose primary action opens the dashboard, symbol `gauge.with.dots.needle.50percent`, labelled "Open the dashboard" for assistive technology, menu indicator hidden) and the Terminal split button; on cards, the terminal split button. Icon-only controls always carry an accessibility label and a help tooltip.
- **Sign In Again:** prominent glass tinted amber when a re-login is needed; plain glass when the login is merely due to expire. While a sign-in is pending it becomes a mini spinner with "Waiting for sign-in…".
- **Borderless:** header icons (SF Symbols in secondary).
- **Mini glass prominent:** "Use" in almanac rows, semibold caption, grey when the account is disabled.

### Chips
- **Plan badge:** 9pt bold uppercase text in the tint colour on the same tint at 18% opacity, capsule, 6pt × 2pt padding. ACTIVE uses cyan text; plan names use secondary; OFF uses grey.
- **Running indicator:** a 6pt Electric Cyan dot beside "N running" in secondary text: in the panel, 4pt from the surrounding caption; in dashboard card badges, 3pt from 9pt semibold "running" / "N running". The count is never cyan text.

### Cards / Containers
- **Corner Style:** 16pt.
- **Background:** flat quaternary system fill; the accent at 18% opacity when selected.
- **Shadow Strategy:** none (see Elevation & Depth).
- **Border:** none, in every state.
- **Internal Padding:** 14pt; 10pt between internal rows. The whole rounded card is the hit target.
- **Content:** mascot and name block, then the card tide reading (28pt figure, 5pt staff, pace delta in caption, week staff), the status or wake note, and the action row.

### Navigation
The dashboard sidebar is a standard source list grouped by domain ("AI", then "HaryX" for app settings), SF Symbol labels, with the brand lockup (20pt tile) inset above the list. Selection uses the system highlight.

### Tide Staff (signature)
The usage reading, and the only usage bar in the app. A flat bar, 6pt tall in the panel and 5pt on cards and in the inspector, 1.5pt corner radius: quaternary track, fill coloured by the Three-Step Fill Rule (for an Off account, one held out of rotation, the cyan step is secondary grey; amber and red stay), and a 1pt primary-colour tick at the expected pace that overhangs the bar by 3pt above and below (it is not clipped by the bar's corners). The 5-hour window always has a pace: the CLI's expected pace when it sends one, otherwise linear over the window from the reset time. Pace text uses a 5-point threshold: at 5 or more points off the pace the line names the delta ("N pts ahead of pace" in amber, "N pts under pace" in secondary); under 5 points the panel says "on pace" in secondary, and only when a pace reading exists. Cards show only the off-pace text for the 5-hour window and stay silent when on pace. Under the 5-hour reading, in the panel's active block and on every card, a Week Staff shows the weekly window: "Week N%" (in the fill colour at 85% and over, otherwise secondary) with "resets in …" on the same caption line, then a 3pt Tide Staff with a linear weekly pace tick. On cards, a caption line under the Week Staff gives the weekly pace from the CLI's expected pace, by the same 5-point threshold: "N pts ahead of pace · runs out Sat 10:15" in amber (the run-out only when the CLI projects the week will not last to its reset), else "N pts under pace" or "on pace" in secondary; the line is reserved and empty while the CLI has no weekly pace (the first day after a reset).

### Usage Bar
The labelled form of the Tide Staff, used in the account inspector: a sentence-case label ("5-hour", "Week") in medium secondary caption with the reset text in tertiary tabular caption on the same line, then a 5pt Tide Staff with the percentage in medium tabular callout in a 40pt trailing column.

### Week Tide (signature)
The week a weekly window has been, drawn on account cards (in place of the 3pt Week Staff; the menu bar panel keeps the staff) and in the Usage page's week rows. x runs from the week's start (its reset minus seven days) to its reset, y from 0 to 100%. A quaternary track at the Tide Staff's 1.5pt radius; the observed use as a filled curve (fill at 32% and a 1.25pt stroke) coloured by the Three-Step Fill Rule (secondary grey in place of cyan for an Off account); a 1pt secondary diagonal from the bottom left to the top right is the linear pace. A stretch with no readings (history started mid-week, a gap, or usage history off) is an estimated straight line: dashed 2/2, fill at 14%. 16pt tall on cards, 22pt in the Usage rows. Drawn at its rendered size, so strokes never scale with the window; nothing animates. Geometry: `WeekTide.segments` (macOS) and `weekTide` (Windows).

### Rotation Verdict
The Usage page's first line, from the CLI's `rotation` in `list --json`: will the accounts auto-switch can use carry you to their resets. "The week lasts" in primary or "Runs dry Sat 10:28" in amber, title-sized, with one secondary line naming the accounts that count and the next reset. Below it, one almanac row per subscription account in switch order: name (middle-truncated, with "Held out of rotation", "Signed out" or "Not used by auto-switch" in secondary caption), Week Tide, weekly %, pace text and reset. A row opens the account's details. Usage's per-account charts give each account one fixed colour from `UsageChartSupport.palette` (deeper `lightPalette` on light backgrounds); none of them is a reading colour (cyan, amber, yellow, lime, red), and no account is recoloured for being active.

### Almanac Row (signature)
One account per row in the "Next in line" table: an 18pt mascot, the name (middle-truncated) with its plan in tertiary caption, then 5h %, 7d % and reset clock in fixed trailing columns, all in tabular 12pt. A disabled account is shown at 50% opacity. Hovering draws one quaternary wash at 50% across the full row, column gaps included (a fill per cell would leave them bare); a click anywhere on the row opens the account menu.

### Desktop Line
Shown in the panel under the active account only while "Claude Desktop per account" is on: a `macwindow` symbol, then one caption line in secondary saying which account Claude Desktop is on ("Desktop: <name>", "Desktop still on <name>", "Desktop: not running", "Desktop: its own folder, not one of HaryX's", or a warning that the active account's folder is signed into another account), 8pt below the block above. At most one mini glass button sits at the trailing edge, only when Desktop is not on the active account: "Open" (the account has a folder) or "Set Up Desktop" (it has none yet). A mismatched folder never gets a button.

Quitting a running Desktop is confirmed with a standard alert, "Claude Desktop will quit and reopen as <email>.", buttons "Quit and Reopen" and "Cancel", and a "Don't ask again" checkbox (Preferences › Claude Desktop turns the question back on).

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

## Web (site/)

The landing page in `site/` (Astro, tokens in `site/src/styles/global.css`) is the one surface outside the macOS app. It keeps the app's brand palette, its display face and its Tide Staff, and replaces the system's glass and appearance with a printed page: a gallery poster with one drawn mark, one tide line and a lot of empty ground. Everything above this section is the app system and still governs the app; this section governs `site/` only, and where the two differ it says so.

### Colors (web)

Brand palette only: Midnight Navy, Electric Cyan and Soft White, with Pace Amber inside the tide illustration. Both themes follow `prefers-color-scheme`, and `data-theme="light|dark"` on `<html>` (set from `?theme=`) forces one.

| Role (CSS token) | Light | Dark |
|---|---|---|
| Ground (`--ground`) | Soft White | Midnight Navy |
| Ink (`--ink`) | Midnight Navy | Soft White |
| Secondary ink (`--ink-2`) | Web Ink 2 | Web Ink 2 Dark |
| Tertiary ink (`--ink-3`) | Web Ink 3 | Web Ink 3 Dark |
| Row rule (`--rule`) | Web Rule | Web Rule Dark |
| List head rule (`--rule-strong`) | Web Rule Strong | Web Rule Strong Dark |
| Live text and strokes (`--live`) | Cyan Ink | Electric Cyan |
| Staff fill (`--live-fill`) | Electric Cyan Light | Electric Cyan |
| Live wash (`--live-wash`) | Web Live Wash | Web Live Wash Dark |
| Staff track (`--track`) | Web Track | Web Track Dark |
| Mark upper half (`--mark-a`) | Electric Cyan | Electric Cyan |
| Mark lower half (`--mark-b`) | Midnight Navy | Soft White |
| Amber figure (`--amber`) | Web Amber Text Light | Pace Amber |
| Amber fill (`--amber-fill`) | Pace Amber | Pace Amber |
| Text selection | Electric Cyan at 45% | Electric Cyan at 35% |

`theme-color` follows the ground (Soft White light, Midnight Navy dark). Body text carries tabular figures by default.

**The Navy Ground Rule (web only).** In the dark theme the whole page ground is Midnight Navy with Soft White ink; in the light theme it is Soft White paper with Midnight Navy ink. This is a deliberate exception, for `site/` only, to the app's rule that Midnight Navy is only the mark's tile. App surfaces keep the system appearance and never use navy as a surface.

**The One Accent Rule (web).** Cyan is the page's only accent: the tide line, the active account and the route in the tide illustration, the `$`/`>` prompt before a command, the selected OS underline, link-hover underlines, the focus ring (2px, 3px offset), and the Copy button's hover and copied fill. Amber appears only inside the tide illustration, as a staff fill and its figure. No red ships on the page.

### Typography (web)

**Display Font:** Space Grotesk (variable, self-hosted via Fontsource)
**Text Font:** Geist (variable)
**Mono Font:** Geist Mono (variable)

- **Web Display** (Space Grotesk 500, up to 6rem, 0.96, -0.04em, max 11ch): the hero headline only.
- **Web Close** (Space Grotesk 500, up to 5rem): the closing "Install HaryX." headline.
- **Web Headline** (Space Grotesk 500, up to 4rem, 1.02, -0.03em): section titles, one per section.
- **Web Figure** (Space Grotesk 500, up to 2.5rem, tabular): percentages in the tide illustration; its reset times use Space Grotesk at text size, tabular.
- **Web Title** (Space Grotesk 500, 1.375rem): platform names.
- **Web Statement** (Geist 450, up to 2rem, 1.2): the one-sentence claim on each "What it does" line.
- **Web Lede** (Geist, up to 1.25rem, secondary ink, 38ch, 46ch in the hero): the paragraph under a headline.
- **Web Body** (Geist 400, 1.0625rem, 1.6): running text; 58 to 62ch where it sets its own measure.
- **Web Small** (Geist, 0.875rem, tertiary ink): notes, requirements, the sample-data line; 0.8125rem for column heads and the footer.
- **Web Command** (Geist Mono, 0.875rem): commands.

The wordmark on the page is not set in type: it is the drawn HARYX stroke lettering (`assets/brand/haryx-wordmark.svg`, 7-unit butt strokes, bevel joins) in the current ink colour, 18px tall in the header and 14px in the footer in secondary ink.

**The Web Display Rule.** On the web, Space Grotesk carries headlines, figures, reset times and platform names, wider than the app's Display-Only Rule, which stays in force for the app. Running text, statements, buttons, the OS switch and nav stay in Geist.

**The Real Command Rule.** Geist Mono is for commands a visitor can run (the install lines, the `haryx …` commands beside each feature, the cswap import line) and nothing else: no mono labels, captions or decorative code.

### Layout (web)

One centred frame (max 1440px) with a fluid gutter (16 to 72px) and a 12-column grid with a fluid gap (16 to 32px). Sections are separated by space alone (96 to 200px), not by bands or fills. Headlines take the left 5 to 6 columns; lists and the illustration run wide or sit to the right. The hero fills the first viewport (min 640px, max 1000px) with the copy at the bottom-left and the mark offset right; the install command is above the fold. Breakpoints at 900px (all columns stack), 760px (the hero mark moves into flow at the top right, 26vh max), 720px (the tide illustration drops its header row and reset column) and 560px (the header keeps only its last link).

### Elevation & Depth (web)

Flat. No shadows, no gradients, no glass, no fills behind content. Depth comes only from stacking in the hero: the mark sits behind the copy and the tide line sits behind the mark.

**The Hairline, Not Card Rule.** Lists are divided by 1px rules: a strong rule at the head of a list, a light rule under each row. Nothing is boxed as a card. The only outlined element is the command field.

### Shapes (web)

Square-cut. 2px corners on the command field and the Copy button, 1.5px on tide staffs, a circle for the account dot, a capsule only for the "active" badge. Curves belong to the water: the tide line and the route are the page's only free curves.

### Components (web)

#### Drawn Mark (signature)
The Wing H's two halves drawn straight on the ground, with no tile: upper half Electric Cyan, lower half in ink (navy on paper, Soft White on navy). In the hero it is tall (360px to 78vh, capped at 860px), anchored top-right in the frame, behind the copy. Entrance: each half is first traced as a 1.25px outline (1.7s, `cubic-bezier(0.65, 0, 0.35, 1)`; upper half at 0.2s, lower half at 0.45s), then floods with its fill from the bottom up (clip-path, 1.1s, `cubic-bezier(0.16, 1, 0.3, 1)`, 1.35s after its trace starts) while the trace fades out. With reduced motion the mark is simply filled. The closing section repeats the mark static, 340px max (120px when stacked).

#### Tide Line (signature)
One full-bleed cyan stroke (2.25px, round caps, non-scaling) running edge to edge of the viewport between the hero headline and the lede, behind the mark. It draws left to right (2.4s, from 1.9s, as the mark finishes), then swells slowly forever (7s, alternate, 4px drop and 0.82 vertical scale). With reduced motion it is static and drawn.

#### Tide Staff Illustration (signature)
A table of three sample accounts that shows auto-switch: rows 84px tall between hairlines, columns for account, 5-hour staff, used % and reset time. Each staff is a 6px flat bar, 1.5px corners, on the track colour, with a 2px ink pace tick overhanging 4px above and below, and a 1px dashed tertiary threshold line overhanging 7px. Under it, "Week N%" in 0.75rem tertiary. When the figure scrolls 60% into view the active account's figure and staff rise from 62% to 91% (1.6s, ease-out cubic), the fill and figure turn amber, and a cyan route curve draws from the active row's dot to the next row, whose dot fills cyan and gains the "active" badge (0.75rem 600, cyan text on the live wash, capsule). A live caption narrates the switch, with account names and the reached percentage in cyan semibold. It must always carry its plain sample-data line ("Illustration with sample accounts, not live data…") and a text Replay link; with reduced motion it jumps to the end state.

**The Sample Data Rule.** Any usage reading on the web is an illustration and says so on the page, next to it, in plain words.

#### Install Switch
The primary action, used in the hero and again at the close, kept in sync across instances and defaulting to the visitor's OS. An OS switch of plain text buttons ("macOS", "Windows", 22px apart, tertiary ink, ink when pressed) with a 2px cyan underline that grows from the left (0.45s) under the pressed one. Below it, one row: the command field (1px strong rule, 2px corners, 14px 16px padding, Geist Mono, a cyan `$` or `>` prompt that is not selectable) and Copy (ink fill, ground text, 2px corners, 48px tall, Geist 500). Copy turns cyan with navy text on hover and for 1.8s after copying ("Copied"); if the clipboard is blocked it selects the command and says "Selected". Under the row, a small tertiary note with the requirements and a line linking the latest release. The closing instance sets the command at 0.9375rem with 18px 20px padding.

#### Captioned Lines
"What it does" is a list, not a grid of cards: each line holds a Web Statement and a secondary caption on the left seven columns and its real commands on the right four, each command led by a cyan `$`. Platforms and terms use the same hairline list, with the name in Web Title.

#### Header and Footer
Header: wordmark left, text links right (What it does, Install, Releases, GitHub) with transparent underlines that turn cyan on hover. Footer: wordmark, project links, and the fine print, above a light rule.

#### Sign-in Return Page
The page the browser lands on after `haryx login` (`cli/crates/haryx-core/assets/callback.html`, served by the CLI on its one-shot `127.0.0.1` listener). It is the same world at poster scale: drawn wordmark top-left, the Drawn Mark top-right (62vh, 30vh under 760px), the headline bottom-left in Web Display, then the Tide Line, then a lede in Web Lede with the account's email in ink. It is answered only once the sign-in has finished, so it never claims success early.

- **Done:** "Signed in." The mark's halves start parted (4.2 × 3.5 user units) and join into the X (1.2s, ease-out), then the cyan tide line draws (1.6s, from 0.75s). Reduced motion shows the joined mark and the drawn line.
- **Failed:** "Sign-in didn't finish." The halves stay parted, a route not made; the line lies flat as a 1px strong rule (no cyan: nothing is live); the lede gives the reason, then `$ haryx login` in a command field.

It makes no request: Space Grotesk 500, Geist 400 and Geist Mono 400 are embedded as subsets (about 9 KB, `assets/fonts`, OFL), the favicon is a data URI, and a CSP of `default-src 'none'` is sent with it. The display subset holds only the two headlines' glyphs; a new headline needs a new subset.

### Do's and Don'ts (web)

#### Do:
- **Do** use the navy ground in dark and Soft White paper in light on `site/`, and only there.
- **Do** let the drawn mark and the tide line be the page's only illustration, with the tide line full-bleed and behind the mark.
- **Do** divide every list with 1px hairlines and leave sections separated by space.
- **Do** label every usage illustration as sample data, in plain words beside it.
- **Do** keep the install command, with its OS switch and Copy, above the fold and repeat it at the close.
- **Do** honour reduced motion: every entrance resolves to its end state, and `?still` renders that end state for captures.

#### Don't:
- **Don't** box content in cards, add shadows, gradients or glass, or fill a section band.
- **Don't** set anything but a runnable command in Geist Mono.
- **Don't** add a second accent: cyan is the only one, amber lives only inside the tide illustration.
- **Don't** present a usage figure as live data.
