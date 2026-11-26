# HaryX — working rules

## Rule: every change lands on every platform

HaryX ships on three surfaces, and a change to one is a change to all of them, in the same change
set. It is never "macOS now, Windows later".

| Surface | Where | Checks |
|---|---|---|
| `haryx` CLI (macOS, Linux, Windows) | `cli/` | `cargo fmt --check`, `cargo clippy`, `cargo test` |
| macOS app (SwiftUI) | `apps/macos/` | `swift build`, `swift test` |
| Windows app (Tauri 2 + Svelte) | `apps/windows/` | `npm run check`, `npm test`, `cargo test` in `apps/windows/src-tauri` |

This applies to behavior, settings, layout and navigation, copy, notifications, and fixes. A
setting that moves to another pane on one app moves on the other. A fix in the CLI is checked
against how each app calls it.

**Why:** the Windows app was added after the macOS one, and changes kept going into macOS
alone. The Windows app then had no Theme setting, a settings layout that no longer matched, and a
tray-mascot preference with no switch to turn it on.

### Before reporting a change done

List each of the three surfaces with what changed there, or with the reason it doesn't apply.
"Not applicable" needs a reason a reader can check, such as "the CLI has no UI". "Not done yet"
is not a reason; it means the task is still open.

### Allowed exceptions

Only when the platform cannot support the feature, or the feature depends on something that has
not been built there yet. Each exception is written down where the next person will find it:

- in `CHANGELOG.md`, on the entry for the change ("macOS only: …, because …");
- in `docs/roadmap.md` if the gap is meant to close later.

Known exceptions today:

- **Claude Desktop per account** (the cards' Set Up/Open Desktop button) is macOS-only. The
  Windows app has no Desktop launcher, and the `haryx ai desktop` folder handling has not been
  checked against Claude Desktop for Windows.
- **Menu bar % reading** is macOS-only; the Windows tray icon has no text.

## Installing a rebuilt macOS app

Install from the DMG (`apps/macos/scripts/package-dmg.sh`, then copy its `HaryX.app` into
`/Applications`), never with `apps/macos/scripts/build.sh --install`. That build has no bundled
`haryx` CLI, so `/usr/local/bin/haryx`, which links into it, breaks.
