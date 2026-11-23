---
version: 1
slug: "apps-macos"
primary_target: "apps/macos"
related_targets: []
---

# HaryX macOS app

Scope: menu bar status icon, menu bar panel, dashboard window, login window, Preferences. Mode: Operate.
Audience and job: developers running several Claude accounts glance mid-task to learn which account is
active, how much of its 5h window is left, when it resets, and what auto-switch will do next.
Constraints: native glass and system Light/Dark; Electric Cyan is the accent; Midnight Navy in the mark and
selected/active emphasis; Space Grotesk for headings, the wordmark and large numbers only, SF Pro elsewhere.
Strict performance budget: ~0% idle CPU with the panel closed, <= 40 MB settled (owner accepted 59 MB on
2026-11-24 as the new baseline: framework-held, measured after 10 panel cycles), no animation while
hidden, flat fills only, render on data change. Measure before and after.
Unresolved: none.

## Direction contract

THESIS: Each account's 5-hour window reads like a tide table: observed use against the predicted pace, the
next reset as the next low water. Refuses the category default of a uniform grouped list of identical bars.

OWN-WORLD: Native glass ground. Soft White text, slate secondary. Electric Cyan owns the active account and
live states; amber for ahead-of-pace and near-limit, red only at limit. Midnight Navy tile for the mark and
selected emphasis. Almanac columns in tabular figures, 1 device-pixel rules, no card borders, flat fills.
Raises: reset countdown as exact length (labanotation); chrome deleted, type carries hierarchy (metro tiles);
1 device-pixel rule precision (centre rail); active account owns the top, the next one staged below (vertical feed).

STORY: The visitor sees which account is live and how far ahead of or behind pace it is, trusts when it
resets and who is next, and switches, opens a terminal or the dashboard in one click.

FIRST VIEWPORT: Panel 380 pt wide. Header row: Wing H mark + HARYX wordmark, auto-switch state. Active block:
account name/plan/running count; 44 pt Space Grotesk 5h %; reset clock right-aligned with countdown; the tide
staff (cyan fill, 1 px white predicted-pace tick); pace delta in amber/cyan plus 7d %; countdown line whose
length is time left. Below a hairline: "Next in line" almanac table (account, 5h, 7d, resets) in switch order.
Last auto-switch event line. Footer: primary "Switch to <next>", Dashboard, New terminal.

FORM: Tide Table, candidate 3 of 7 on the ordered list; seed:8e83db1f0cfa6388.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
