# Changelog

All notable changes to HaryX (the `haryx` CLI and the macOS app) are recorded here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

Each version's section becomes its GitHub release notes (`scripts/release/notes.sh`), so a tag
cannot be released without one. Sections, in this order, only when they have entries:
**Breaking changes** (what you must change, and how), **Added**, **Changed**, **Deprecated**,
**Removed**, **Fixed**, **Security**.

## [Unreleased]

### Added

- macOS app: "Claude Desktop follows Use" (Preferences › Claude Desktop, off by default). Pressing Use
  quits a running Claude Desktop normally and reopens it on that account's own Desktop folder, so
  Desktop and Claude Code sign in as the same account. Each account signs in to Desktop once with
  Set Up Desktop in the menu bar panel, whose Desktop line also shows when Desktop was left on
  another account (auto-switch and AI Wake never quit Desktop). Asks before quitting, with "Don't
  ask again". A folder signed into another account is left alone. Each folder takes about 750 MB.
- `haryx ai desktop status [--json]` and `haryx ai desktop path ACCOUNT`: each account's Claude
  Desktop data folder, keyed by account UUID, and which account it is signed into.

- macOS app: a "Not saved" notice in the bottom-right corner of the Config and Auto-switch panes
  while a typed setting has not been saved with Return, so a value like the auto-switch threshold
  is no longer silently lost. Esc puts the field back to the stored value.

## [0.1.2] - 2026-11-24

### Added

- `scripts/install.sh`: installs or updates the macOS app with one `curl … | sh` command. It checks the
  DMG's SHA-256, replaces an older copy safely and opens the app without the Privacy & Security
  approval a browser download needs.
- `haryx ai wake`: starts an account's next 5-hour window right after the last one resets, with one
  1-output-token request. Opt in per account (`on`/`off`, or `all`), set quiet hours and a weekly cap
  (`config`), and install a LaunchAgent that checks every minute (`install`). Checks with nothing due
  read local files only; a failed login sends one macOS notification.
- macOS app: Wake controls — a Wake pane (background check, quiet hours, weekly cap, per-account
  switches) and a Wake section in each account's inspector.
- macOS app: the menu bar panel's Dashboard button has a dropdown that opens the dashboard on any
  section (AI: Accounts, Auto-Switch, Wake, Folder Mappings, Setup Profiles, CLI Settings, Backup &
  Maintenance; HaryX: App Preferences).
- macOS app: refresh buttons (menu bar, Accounts, Setup Profiles) spin and disable themselves while
  a refresh runs; Reduce Motion pulses instead.
- macOS app: hovering a "Next in line" row in the menu bar panel shows its full email at once in a
  small label under the row (above it for the last row); hovering the active account shows its
  email (and alias) as help.
- macOS app: an "All accounts" switch at the top of the Wake page's account list turns wake on or
  off for every account at once (`haryx ai wake on all` / `off all`) and shows how many are on.
- macOS app: a weekly bar (Week %, its reset and a pace tick) under the 5-hour reading on every
  dashboard card and on the active account in the menu bar panel.

### Changed

- macOS app: redesigned to the HaryX brand. The menu bar panel reads the active account like a tide
  table (large 5-hour %, reset clock, a flat usage staff with a pace tick, time left drawn as length)
  over a "Next in line" table in switch order, with a one-click "Switch to …". Dashboard cards follow
  the same reading. Space Grotesk (bundled, OFL) for the wordmark and large numbers, a deep Electric
  Cyan app accent, flat fills instead of gradients.
- macOS app: the menu bar shows the Wing H mark (with a dot while the active account runs a
  session) by default. The animated mascot is now opt-in: Preferences → "Show animated mascot in
  the menu bar" (off by default; its walk timer runs only while the active account runs a session).
- macOS app: idle cost cut — mascots stop while the dashboard is covered or minimized, and the
  session scan slows to every 15 s while nothing is on screen. Measured idle: ~0.1% CPU and 22 MB
  footprint on a fresh launch (was ~6% and 111 MB on a long-running build).

### Fixed

- macOS app: opening the dashboard while it is minimized brings it back.
- macOS app: the hover highlight in the menu bar panel's "Next in line" table covers the whole row,
  and a click anywhere on the row opens the account's menu. Rows have a small side inset, and each
  one shows a visible "Use" button instead of a hover-only text link.
- macOS app: the dashboard title bar keeps one height in every section, so the window buttons and
  content no longer shift when you change sections.
- macOS app: the thin cyan time-left line under the 5-hour bar is gone from the panel and the cards.
- `haryx ai wake`: an account whose wake started a window is not woken again every 15 minutes
  while the usage reading still shows 0% with no reset time; the reset from the wake's reply is
  kept until it passes. Turning wake off and on again no longer skips the 15-minute retry wait.
- macOS app: the menu bar panel's Resets column shows the reset of a window a wake just started
  instead of "not started".
- macOS app: the menu bar panel closes when you click another app, switch apps (⌘-Tab, Dock) or
  bring the dashboard forward, like system menu bar panels.
- macOS app: the whole account card is clickable, including its edges and empty space.
- macOS app: account cards keep one height when an account needs a re-login.

## [0.1.1] - 2026-11-23

### Changed

- Account, usage and session commands now live in the `ai` section: `haryx ai list`,
  `haryx ai switch 2`, `haryx ai tui`, `haryx ai watch`, and so on. `haryx --help` lists the
  sections; `haryx ai help` lists the commands. Hints and help text use the `ai` form.
  The cswap spellings without `ai` (`haryx list`, `haryx --list`) keep working, so scripts written
  for `cswap` are unaffected. Bare `haryx` in a terminal still opens the dashboard.
- macOS app: the menu bar panel and the dashboard sidebar group everything about AI coding
  accounts under an **AI** section; the sidebar's app preferences sit under **HaryX**.

### Added

- `haryx ai profile` moves a Claude Code setup between machines: `list`, `show`, `save`, `export`,
  `import`, `apply`, `diff`, `delete`, `rollback`. A profile carries skills, hooks, MCP servers,
  plugins (`--plugins ref|bundle`), memory files, other `~/.claude` entries and history
  (`--no-history` to leave it out); credentials, signed files, the whole `~/.claude.json` and caches
  are opt-in. Home paths are made portable, secrets are redacted or encrypted with a passphrase
  (`--secrets redact|encrypt`), imported files are checked entry by entry, everything that can run
  code is listed for confirmation, `--prune` removes local extras (never history), and every apply
  is backed up for `rollback`.
- macOS app: **Setup Profiles** section to browse, save, export, import and apply profiles, with
  the dry-run plan shown before applying.
- macOS app: the DMG build offers on first launch to install the bundled `haryx` command in
  `/usr/local/bin` or `~/.local/bin` (Preferences › Terminal command later).

- `haryx ai login [NUM|EMAIL|ALIAS] [--slot N] [--alias NAME] [--sso]`: signs a managed account in
  again (fixes "re-login needed") or adds a new account, by running Claude Code's own
  `claude auth login` in an isolated profile. The active login is left untouched. It exists only as
  `haryx ai login`: the top-level `haryx login` is kept for a future HaryX account.
- macOS app: **Sign In Again** on accounts that need a re-login, and **Sign In New Account**, in
  the account inspector, the menu bar Terminal dropdown and the dashboard toolbar. The menu bar's
  AI section has a **+** button that signs in a new account.
- macOS app: sign-in runs in the background in its own window, which shows the sign-in link
  (Open / Copy), a field to paste the code, the live output and the outcome. Closing the window
  cancels the sign-in.

### Fixed

- The "re-login needed" hint now points at `haryx ai login` instead of `haryx add`.

## [0.1.0] - 2026-11-23

First release: the `haryx` CLI (a Rust port of claude-swap, drop-in compatible with `cswap`) and
the HaryX macOS menu bar app.

[Unreleased]: https://github.com/conghai20/haryx/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/conghai20/haryx/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/conghai20/haryx/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/conghai20/haryx/releases/tag/v0.1.0
