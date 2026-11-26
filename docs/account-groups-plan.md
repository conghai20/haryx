# Account groups

Status: built on branch `feat/haryx-account` (2026-11-26): CLI `3097467`, macOS `c169ee1`, Windows
`80a97a6`. Awaiting a hands-on check in both apps.

## Goal

Let the user sort accounts into named groups. The Accounts dashboard shows one section per group,
and automatic switching prefers accounts in the active account's group.

## User-facing flow

In an account's inspector, the user types a group name next to the alias; existing names are
suggested. The Accounts dashboard then shows collapsible sections: the active account's group
first, other groups alphabetically, "Ungrouped" last. Automatic switching (rotation, `best`,
`next-available`, and the auto-switch engine) picks from the active account's group first, where
"Ungrouped" is a group of its own. When nothing in that group can take over, switching falls back
to the other accounts and says so. Picking an account by hand works as before. With no groups set,
nothing changes.

## Decisions (from the owner)

| Topic | Decision |
|---|---|
| Purpose | Dashboard sections, plus group-scoped switching, plus grouping in the tray and menu bar. |
| Storage | A `group` field on the account record, next to `alias`: synced, exported, CLI-settable. |
| Membership | Several groups per account (changed 2026-11-26 from one: a second group replaced the first, which read as a bug). Stored as `groups`, with `group` = the first for older readers. Accounts sharing any group count as the same group. |
| Assigning | A Group field in the inspector; collapsible sections whose collapsed state is kept per machine. |
| Group exhausted | Fall back to the other accounts. |
| Active account ungrouped | Prefer other ungrouped accounts ("Ungrouped" acts as a group). |
| Toggle | Always on. Assigning groups is the opt-in. |
| Which picks | All automatic picks: rotation, `best`, `next-available`, auto-switch. Manual picks are not scoped. |

## The field

`group`: a string, trimmed, 1–32 characters. Matching ignores case, and typing a name that already
exists in another case joins the existing group with its stored spelling. An empty value or
`--unset` clears it. A group exists only while some account carries it.

## Tasks

1. **CLI: store the group.** `[cli]` Add `group` to `USER_ACCOUNT_SETTINGS` (sync and carried
   settings), validate it in `switcher/synced.rs`, carry it through `transfer.rs`
   export/import, and show it in `list` and `list --json`. New command
   `haryx group <num|email|alias> <name> | --unset`, written the same way as `alias`.
   Tests: set and clear, case-insensitive join, round trips through sync and export.
2. **CLI: prefer the group when switching.** `[depends: 1]` One helper in `switcher/` puts
   same-group candidates first and the rest after, used by:
   - `select_best_switchable`: score the group first, and the rest only if the group has nothing;
   - rotation and `next-available`: the group in sequence order, then the others;
   - `autoswitch/tick.rs`: rank same-group candidates first, rank the rest if that gives no
     switch, and emit a "left group" detail when it falls back.

   Tests: each strategy stays in the group, falls back when the group is exhausted or disabled,
   and treats ungrouped accounts as a group.
3. **macOS app.** `[depends: 1]` `group` on `Account`. `nextInLine` puts the active account's
   group first, so the menu bar's "next" agrees with the CLI. `AccountsPane` shows collapsible
   sections, with collapsed state in UserDefaults and never synced. A Group field in
   `AccountInspector`. Group headers in the menu bar's account list. Tests: `swift test` for
   section building and `nextInLine` ordering.
4. **Windows app.** `[depends: 1]` `group` in `types.ts`; `store.others` puts the group first;
   collapsible sections in `Accounts.svelte`, with collapsed state in localStorage; a Group field
   in `AccountInspector.svelte`; group headers in the flyout list; `actions.setGroup`. The tray's
   "Use next" goes through the CLI and so follows the group. Tests: vitest for grouping and
   ordering.
5. **Docs.** `[depends: 2, 3, 4]` A `CHANGELOG.md` entry, CLI help and the epilog, and
   `docs/roadmap.md` if groups are listed there.

Tasks 3 and 4 are independent of each other once `list --json` carries `group`.

## Verification

- CLI: `cargo fmt --check && cargo clippy && cargo test` in `cli/`.
- macOS: `swift build && swift test` in `apps/macos/`.
- Windows: `npm run check && npm test` in `apps/windows/`, and `cargo test` in
  `apps/windows/src-tauri`.
- By hand: rebuild the macOS DMG, install it, and check grouping and switching with real accounts.

## Risks

- An auto-switch run started before the update doesn't know about groups until it restarts.
- An older app on another machine syncs without the `group` field. Check that this doesn't clear
  the field on the newer machine.

## Out of scope

Reordering groups by hand, groups that exist with no accounts, a setting to turn scoping off, and
colors or icons for groups.
