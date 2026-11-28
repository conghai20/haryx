---
version: 1
slug: "apps-platform"
primary_target: "apps/platform"
related_targets: []
---

# HaryX web (apps/platform)

Scope: the signed-in web at haryx.wecoap.com/platform/ plus its way in (WeCoAp sign-in, sync passphrase unlock). Mode: Operate.

Audience and job: developers with several Claude accounts, plus first-timers. Main visit: account overview (who was used last, how full each account is, which machines). Also usage history, profiles, machines, privacy (consents, export, erase). Desktop and phone equally.

Constraints: same HaryX world as site/ (Soft White / Midnight Navy grounds, Geist, Space Grotesk figures, Geist Mono commands, X mark, tide line, square 2px corners). Components from shadcn-svelte (owner request), themed to that world. CSP is style-src 'self' and script-src 'self'. Web data is synced readings, never live: no "in use now", no rotation verdict, no pace or reset (sync carries none); every figure says when and from which machine it was read. Steer from owner: high-end minimal SaaS, creative, inheriting haryx.wecoap.com and wecoap.com.

## Direction contract

THESIS: The homepage hero becomes the console: the account you used last stands at display size on the left; every other account is a calm row on the right. Refuses the grey admin sidebar with boxed cards.

OWN-WORLD: Soft White paper (light) / Midnight Navy (dark) ground, navy ink, hairline rules, 2px corners, no shadows or cards. Space Grotesk 500 at display size for the hero account and figures; Geist body; tracked Geist Mono eyebrows (wecoap.com). Cyan only on the last-used marker and on-pace fills; amber at 85%+, red at 100%. The X mark large and cropped in the hero panel; the homepage tide line crosses both panels.

STORY: The visitor signs in with WeCoAp, unlocks with the sync passphrase, sees at once which account they used last and how full every account is, and moves to Usage, Profiles, Machines or Privacy from the tabs.

FIRST VIEWPORT: Desktop split 40/60. Left (sticky, full height): wordmark; eyebrow "Last used · <machine> · <time>"; account name ~88px Space Grotesk; email; 5-hour figure ~64px with Tide Staff; week figure with thin staff; synced summary; X mark cropped bottom-left; user + sign out at bottom. Right: section tabs + search (Cmd+K) on top; eyebrow "Accounts · N"; hairline rows with name, 5h figure + staff, week %, machines, chevron opening a side sheet. Phone: hero collapses to a compact header block above the list; tabs scroll.

FORM: Split hero, position 6 on my ordered list of grounded structures (re-roll round 1), seed: d5dc91adb11fa5aa. Signature move: the tide line crossing from the hero into the account list.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
