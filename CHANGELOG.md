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

- `haryx login`, `haryx logout` and `haryx whoami` (each with `--json`): sign this machine in to a
  HaryX account with GitHub or Google (`--provider google`), for sync between machines
  (coming). The browser opens and returns to a HaryX page that says who signed in, or why the
  sign-in did not finish; `--no-browser` prints the link instead. The session is kept in the
  Keychain on macOS and in a private sealed file elsewhere. Claude accounts are untouched: they are
  still signed in with `haryx ai login`, and `haryx login 3` now says so.
- `haryx sync`: sync between your machines, end-to-end encrypted with a sync passphrase (`sync unlock`;
  `sync passphrase` changes it). Carries `haryx config` settings (except `history.import`), the apps'
  preferences (`sync prefs get|set`), each Claude account's alias, group, priority, threshold and enabled state,
  and API-key and setup-token accounts. Claude browser logins never leave the machine: accounts that
  exist only elsewhere are listed by `sync status` to sign in here. `sync forget EMAIL`, `sync reset`.
- `haryx sync profile push|pull|list|remove`: setup profiles on the server, encrypted (secrets
  redacted, no history; three at most).
- `haryx account delete`: deletes the HaryX account and everything synced.
- macOS and Windows apps: a HaryX Account pane (sign in, unlock, sync, change passphrase, sign out,
  delete), automatic sync every 15 minutes and after a synced preference changes, and "On your other
  machines" in Accounts.
- macOS and Windows apps: HaryX sign-in lives in the dashboard sidebar's footer, a dropdown of the
  ways to sign in (GitHub or Google). Signed in, the footer shows the account and
  sync state, with Account & Sync and Sign Out in its menu.
- Account groups: `haryx group <num|email|alias> <name>` (`--unset` to remove, no account to list).
  Names are up to 32 characters and match ignoring case. Automatic switching (rotation, `best`,
  `next-available` and `haryx auto`) prefers accounts in the active account's group, ungrouped
  accounts counting as a group of their own, and leaves it only when none there can take over,
  saying so in its warnings; `switch <num>` is unaffected. The group shows in `list` and
  `list --json`, and travels with export/import and sync. In both apps the Accounts pane shows a
  collapsible section per group, the inspector has a Group field, and Next in line and "Claude
  as…" are grouped.

## [0.1.6] - 2026-11-25

### Added

- macOS app: a Sessions section (after Usage) listing every running Claude Code session across accounts, newest first, with its account, folder, pid, how long it has run, "out of quota" when it is stuck, and Continue on…, plus a Run Claude button per account, as on Windows.
- macOS app: Wake shows each account's last wake, next due and last result, and the recent wake events, as on Windows.
- macOS app: a Diff button on a saved setup profile, comparing it with the live `~/.claude`, as on Windows.
- macOS app: Import from cswap in Backup & Maintenance (haryx backend only), as on Windows.
- macOS app: Hide routine checks over the auto-switch log, on by default, as on Windows.
- Windows app: Warp as a terminal choice, as on macOS. It opens through a launch configuration in
  Warp's data folder and runs in Warp's own shell (PowerShell unless changed in Warp).
- `haryx ai doctor` (and `--json`) says whether Claude Code is installed and signed in, and how to
  fix it; `claude` is also found outside PATH (the native installer's `~/.local/bin`,
  `~/.claude/local`, Homebrew and `/usr/local/bin`, and npm's folder on Windows).
  Both apps show a banner on Accounts and in the menu bar panel / tray flyout while something is
  wrong, with Install Claude Code (a terminal running Anthropic's installer: `install.sh` on macOS,
  `install.ps1` on Windows), Open Claude to Sign In, and Check Again.
- Both apps: a Theme setting (System, Light, Dark). macOS: App Preferences, applied to the
  dashboard and the menu bar panel. Windows: Preferences, applied to the title bars and both windows.
- Windows app: Sign In Again and Sign in new account open HaryX's own sign-in window, as on macOS (the sign-in link with Open and Copy, live status, a field for the code the sign-in page shows), instead of a terminal. A terminal is offered only if that window cannot start.
- Windows app: a switch for the tray mascot (Show the active account's mascot in the tray), which
  had a preference but no way to turn it on.
- macOS only (Windows has no Desktop launcher yet): each account card has a Claude Desktop button, Set Up Desktop until that account has
  signed in to Desktop, then Open Desktop. The account menus offer Set Up Claude Desktop too. Neither
  switches Claude Code.
- Windows app: pixel mascots on every account card, the tray flyout's rows and the inspector, with
  the macOS moods (walking while Claude runs, tired at 90%, asleep when held out, "!" when signed
  out) and a per-account species and colour picker; the tray mascot uses the same choice. Both
  apps' folder mappings lists show them too.
- Windows app: a Folder mappings pane (AI group, after Wake), as on macOS: mapped folders with
  their account's mascot, Run Claude in one, Unmap, and Choose folder and map… for an account.
  The tray flyout's terminal menu lists mapped folders, one click runs Claude there.
- Windows app: Move to slot and Swap with… in the account inspector, as on macOS.
- Windows app: Force Switch, as on macOS: in the inspector (it asks first) and in the flyout's
  account menu.
- Windows app: Run Claude in folder… in the inspector (as that account) and Claude in folder… in
  the flyout's terminal menu (the active account), as on macOS.
- Windows app: clicking an account in the tray flyout opens its menu, as on macOS: Info (the
  dashboard with that account selected), Sign In Again…, Switch, Force Switch, Remove….
- Windows app: Sign In New Account… in the flyout's terminal menu, as on macOS.
- Windows app: a "Claude session finished" notification for sessions that ran 1 minute or
  longer, on by default, with its switch in AI settings, as on macOS.
- Windows app: Open data folder and Show log (`haryx.log`, selected in Explorer) in Tools, as on
  macOS.

### Changed

- macOS app: CLI Settings is now AI Settings and also holds the usage refresh, menu bar reading and
  mascot, Terminal, Claude Desktop and Notifications settings, which moved out of App Preferences.
  App Preferences keeps launch at login, the terminal command and the CLI path.
- Windows app: Settings moved into the AI group as AI settings (usage refresh, tray mascot,
  terminal, start folder, notifications, haryx settings). A new HaryX › Preferences keeps start with
  Windows, Theme and the haryx path.

### Fixed

- macOS app: a Desktop folder that exists but never signed in offers Set Up Desktop, not Open.

## [0.1.5] - 2026-11-24

### Added

- A project site (Astro, in `site/`) deployed to GitHub Pages: what HaryX does, an animated
  roadmap, platforms, and a one-line install for macOS and Windows.
- Usage history. `haryx ai usage [report|quota|summary]` shows tokens and what they would cost at
  API prices (as of a dated price table) by account, project, session, model or day, and the
  5-hour/weekly readings over time, read from Claude Code's transcripts and kept on this machine
  (no conversation text). The first run asks whether to read past transcripts or start from now;
  `history.keepDays` (30, 90 by default, 365, or 0 for forever) sets how long it is kept. The
  macOS app adds AI › Usage: quota-over-time and per-day charts, top projects and sessions, and an
  optional weekly summary notification; the Windows app adds the same Usage pane (SVG charts) and
  weekly toast.
- Continue a session on another account. `haryx ai sessions continue <pid|id> [--to N|best]` ends
  a Claude Code session (a normal termination request, waiting up to 10 s; on Windows you close
  the window) and resumes the same conversation on another account in its folder. Both apps add
  "Continue on…" to each running session, mark sessions stopped on an exhausted account, and show
  "N sessions stopped on account X — Continue on Y" in the menu bar panel / tray flyout. After an
  auto-switch the macOS app also notifies about sessions left on the exhausted account.

- macOS app: Export… on an account card's right-click menu opens Maintenance › Export with that
  account ticked. The export list there is now a checklist, so any set of accounts can go in one
  file (Select All / Select None).
- `haryx ai export PATH --account 1,3` takes several comma-separated numbers or emails.
- macOS app: HaryX › About & Updates shows the app and CLI versions and the latest release notes,
  and updates HaryX in place. Choose Install automatically (download in the background, install on
  quit or Restart Now), Notify me (the default: a daily check and an "is available" row in the menu
  bar) or Only when I check; Skip This Version hides one release. Each download is checked against
  its SHA-256 and the app's signature first; the bundled haryx updates with the app. When HaryX
  cannot write to its own folder it opens the disk image in Finder instead.
- Windows app: HaryX › About & updates, with the same three update modes, release notes, and an
  "is available" row in the tray flyout. The checked installer runs silently once HaryX quits (a
  per-user install, no admin prompt) and HaryX reopens.
- `haryx upgrade --check [--json] [--refresh]` reports the latest GitHub release: its version,
  notes and the app and CLI downloads for this machine. The apps use it for their own updates.
- `haryx upgrade` updates a standalone install (a release tarball unpacked by hand) in place: it
  downloads this platform's tarball, checks its SHA-256 and that the new binary reports the new
  version, then swaps it in. The haryx inside the HaryX app says it updates with the app.
  `haryx upgrade --download-app DIR [--json]` downloads and checks this machine's app installer
  (the Windows app uses it).

### Changed

- macOS app: auto-switch keeps its on/off state across quit and relaunch. Left on, it starts again
  when HaryX reopens; "Always start when HaryX launches" still forces it on.

## [0.1.4] - 2026-11-24

### Added

- macOS app: "Claude Desktop follows Use" (Preferences › Claude Desktop, off by default). Pressing Use
  quits a running Claude Desktop normally and reopens it on that account's own Desktop folder, so
  Desktop and Claude Code sign in as the same account. Each account signs in to Desktop once with
  Set Up Desktop in the menu bar panel, whose Desktop line also shows when Desktop was left on
  another account (auto-switch and AI Wake never quit Desktop). Asks before quitting, with "Don't
  ask again". A folder signed into another account is left alone. Each folder takes about 750 MB.
- `haryx ai desktop status [--json]` and `haryx ai desktop path ACCOUNT`: each account's Claude
  Desktop data folder, keyed by account UUID, and which account it is signed into.

- HaryX for Windows: a tray app over the `haryx` CLI (Tauri 2). The flyout shows the active account's
  5-hour reading and the next accounts with Use; the dashboard has accounts, running sessions,
  auto-switch with its live log, wake, setup profiles, tools and settings. Toasts for switches,
  usage, sign-in and resets (the macOS rules), launch at login, terminals (Windows Terminal,
  PowerShell 7, Windows PowerShell, Command Prompt, Git Bash), and the account mascot in the tray.
  Installs per user with `irm .../scripts/install.ps1 | iex`; winget manifests are in each release.
- The `haryx` CLI runs on Windows: the store is `%LOCALAPPDATA%\haryx`, saved logins are sealed
  with DPAPI for your Windows user, folders are shared through junctions (files through symlinks
  when Developer Mode allows), `claude` is found through PATHEXT (`claude.exe` or npm's
  `claude.cmd`), and `haryx ai wake install` registers a per-user Task Scheduler task. Releases
  carry x64 and ARM64 Windows binaries, wheels and npm support.
- `haryx ai sessions [--json]`: every running Claude Code session and the account it runs as, read
  from Claude Code's own session records (contract: `packages/contracts/haryx/sessions.json`).
- `haryxw`, a build of haryx without a console window (Windows releases only), used by the wake
  task. Leading `--env NAME=VALUE` pairs set the store variables a scheduled task cannot carry.

- macOS app: a "Not saved" notice in the bottom-right corner of the Config and Auto-switch panes
  while a typed setting has not been saved with Return, so a value like the auto-switch threshold
  is no longer silently lost. Esc puts the field back to the stored value.

### Fixed

- A profile rollback on Windows refused to run instead of deleting a link it could not put back.
- Included folders in setup profiles were refused on Windows as "outside your home folder" because of
  the drive prefix.

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

[Unreleased]: https://github.com/conghai20/haryx/compare/v0.1.6...HEAD
[0.1.6]: https://github.com/conghai20/haryx/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/conghai20/haryx/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/conghai20/haryx/compare/v0.1.2...v0.1.4
[0.1.2]: https://github.com/conghai20/haryx/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/conghai20/haryx/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/conghai20/haryx/releases/tag/v0.1.0
