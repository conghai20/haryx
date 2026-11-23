# Claude Desktop follows the Use button

Status: planned (2026-11-24). Blocked on the Phase 0 sign-in test. No code yet.

## Goal

When the user presses Use on an account in HaryX, Claude Desktop switches to the same account. Today a
switch changes only Claude Code's login (Keychain credentials and `oauthAccount` in `.claude.json`).
Desktop keeps its own claude.ai web login, so it stays on whatever account last signed into it.

## Approach

Each account gets its own Desktop data folder. On Use, HaryX asks Desktop to quit normally, then
reopens it with `--user-data-dir=<that account's folder>`.

- No symlinks, no copying or reading cookies, no re-signed copies of Claude.app.
- Nothing is ever deleted.
- The account a folder is signed into is read from `lastKnownAccountUuid` in the folder's
  `config.json`. It equals Claude Code's `oauthAccount.accountUuid` for the same account.

## Decisions (from the owner)

| Topic | Decision |
|---|---|
| Mode | Follow the switch. One Desktop at a time: quit, then reopen on the new account's folder. |
| Trigger | Manual Use only. Auto-switch and AI Wake never quit Desktop; they show a hint instead. |
| Desktop running | Ask each time, with a "Don't ask again" option. |
| Old ClaudeHarbor data | Planned as copy-in; dropped after the Phase 0 copy test failed (see below). |
| Default | Opt-in setting, off by default. |

## Phase 0 findings (Claude Desktop 4.23118.0)

Bundle id `com.anthropic.claudefordesktop`.

Verified:
- `open -a /Applications/Claude.app --args --user-data-dir=<dir>` opens Desktop on that folder only.
  The default `~/Library/Application Support/Claude` is not created.
- A normal quit (AppleScript `quit`) finishes in about 2 seconds.
- `config.json` holds `lastKnownAccountUuid`, and it matches the Claude Code account UUID.
- A signed-in folder is about 734 MB.

Failed:
- A copy of the ClaudeHarbor Desktop folder, opened with the real Claude.app, showed the sign-in
  screen. The log said `[oauth-v2] failed to load token cache`. The cause is either a key from
  ClaudeHarbor's re-signed app copy or an expired login; telling them apart would mean decrypting the
  login, which this feature must never do. So folders are created in place and never moved or copied.

Still unproven (gate for Phase 1):
1. Sign into Desktop in an empty folder with the real app; the browser must return to the app through
   the `claude://` link.
2. Quit and reopen on the same folder: Desktop must still be signed in, with the same UUID.
3. The updater and the single-running-copy check behave normally with a custom folder.

If any of these fails, stop and rethink before writing code.

## Phase 1: CLI core (Rust, `haryx-core/src/desktop/`)

- Folder location: `~/Library/Application Support/HaryX/desktop/<accountUuid>/`. Keyed by UUID, not
  email or account number, because numbers can be reassigned.
- `signed_in_uuid(folder)`: reads `config.json`; a missing file, missing key or bad JSON is "unknown".
- Commands:
  - `haryx desktop status --json`: per account, whether a folder exists, its signed-in UUID, and
    whether that UUID mismatches the account.
  - `haryx desktop path <account>`: the folder the app should launch with.
- Tests first: config parsing, unknown vs mismatch, path keying.

## Phase 2: macOS app

- `Services/DesktopLauncher.swift`:
  - Finds the running Desktop by bundle id.
  - Asks it to quit normally and waits up to 10 seconds. Never force-kills; if Desktop does not quit,
    the relaunch is cancelled and the user is told why.
  - Reopens Desktop with the account's folder.
- Hook: runs only after a successful `switchTo` (`AppStore+Actions.swift`). `AutoSwitchRunner` and AI
  Wake never call it.
- Confirm sheet when Desktop is running: "Claude Desktop will quit and reopen as b@x.com."
- Setting: "Claude Desktop follows Use", off by default.
- Menu bar status line:
  - "Desktop: a@x.com".
  - "Desktop still on a@x.com" after an automatic switch.
  - "Set up Desktop" for an account with no folder: opens Desktop on a new empty folder so the user
    signs in once.
- Mismatch guard: if a folder is signed in as a different account, warn and leave the folder alone.

## Phase 3: Finish

- DESIGN.md and CHANGELOG for the status line, setting and confirm sheet.
- `cargo test`, `swift test`, code review.
- Built on its own branch; merged after review.

## Out of scope

Cookie handling, symlinks, cloned or re-signed app copies, several Desktops at once, following
automatic switches, importing ClaudeHarbor data.

## Risks

- A Desktop update could drop `--user-data-dir` support. If launching with the folder fails, open plain
  Desktop and show a warning.
- Disk: about 734 MB per account folder.
- `lastKnownAccountUuid` is undocumented. A missing value is "unknown", never a mismatch.
