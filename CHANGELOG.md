# Changelog

All notable changes to HaryX (the `haryx` CLI and the macOS app) are recorded here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

Each version's section becomes its GitHub release notes (`scripts/release/notes.sh`), so a tag
cannot be released without one. Sections, in this order, only when they have entries:
**Breaking changes** (what you must change, and how), **Added**, **Changed**, **Deprecated**,
**Removed**, **Fixed**, **Security**.

## [Unreleased]

## [0.3.0] - 2026-11-28

### Breaking changes

- HaryX accounts moved from Supabase to WeCoAp SSO (`sso.wecoap.com`) and the HaryX API
  (`haryx.wecoap.com`), both in Vietnam. Nothing migrates from 0.2.0's Supabase sign-in: a
  Supabase-era session is signed out and asks you to sign in again with your WeCoAp ID. `HARYX_SUPABASE_URL`
  and `HARYX_SUPABASE_KEY` are gone; `HARYX_SSO_ISSUER` and `HARYX_API_URL` point a build at a
  local stack. `haryx account delete` no longer deletes: it shows where to delete your WeCoAp ID.
  `haryx login --provider` is gone: sign-in always goes through the WeCoAp SSO page, which offers
  every method. Drop the flag from scripts.

### Added

- Sign in with WeCoAp SSO, the only way in: `haryx login` (`--no-browser` for the device grant) and
  one "Sign in with WeCoAp" button in both apps. The WeCoAp page offers GitHub, Google, Apple, and
  email and password.
- Consents, per Law 91/2025: Sync, and showing your machines on HaryX web, each asked before
  first use and withdrawable; withdrawing erases that purpose's data on the server. `haryx account
  consents`, `haryx account consent grant|withdraw sync|machines`, `haryx account export`,
  `haryx account machines`, `haryx account machine rename`. Both apps: consent sheets, a Privacy
  section (machine toggle, Withdraw, Export, policy link), and "Open HaryX on the web".
- Usage syncs end-to-end encrypted: one monthly summary per machine (tokens by day, account and
  model, plus 5h/7d readings; never a project path, session id or conversation text). The Usage
  pane says "It stays on this machine" only when it does not sync.
- HaryX web (`https://haryx.wecoap.com/platform/`): Machines (rename, sign out) and the Privacy
  center; Accounts, Usage and Profiles once you enter your sync passphrase in the browser, which
  the server never sees.
- "Signed out of HaryX from the web" in both apps when a machine is signed out remotely.
- HaryX web checks for a WeCoAp SSO session silently, in a hidden frame: a signed-in browser opens
  straight on the accounts, without a trip through the SSO page; anyone else gets the sign-in page.
  A check that gets no answer in 15 seconds offers Try again instead of waiting.
- The landing site links into HaryX web (Sign in in the header, a Web entry under "Four ways in",
  the footer) and its roadmap and feature list match this release: sync and HaryX web are shipped.

### Changed

- The landing site moved from `site/` to `apps/landing/`, where future landing pages will live.

- Set up panes, both apps. AI settings runs by task: usage (the CLI's usage readings now sit with
  the app's own usage settings), notifications, usage history, terminal, run, Claude Desktop, then
  one "Command line" section for display and updates. Choice settings read as words ("Auto",
  "Skip them") instead of raw values. Setup profiles explains what a profile is until one exists,
  labels its toolbar buttons, folds each category to a count and size (a category with a problem
  starts open), shows a repeated hook once with ×N, and offers Undo Last Apply only after an apply.
  Backup & maintenance folds the export accounts into one "Accounts" row ("All 20", "3 of 20")
  that opens to checkboxes, so a long account list no longer pushes Export down; it splits Import into accounts and
  usage, picks the usage hold time from a list instead of a 5-minute stepper, explains unclaimed
  credentials, and marks its purge buttons red. The CLI's version, executable, upgrade, data
  folder and log moved to About & updates. Windows keeps sentence-case buttons and has no Claude
  Desktop section, as before.

- Auto-switch `best` strategy: when candidates are within `hysteresis_pct` of the one with the most
  headroom, it now takes the one whose weekly window resets soonest, so quota that would reset
  unused is spent first. A clearly roomier account still wins. CLI only: both apps run `haryx auto`
  and pick up the change; their "Next in line" order stays a usage-% approximation.

- Menu bar panel (macOS) and tray flyout (Windows): an unstarted 5-hour window reads "starts on
  first use" instead of a bare dash, and the week line turns amber when the week is the tighter
  limit (over 50% and above the 5-hour). "Next in line" names each group once (switch order inside
  it), folds accounts held out of rotation into a "Held out (N)" line at the bottom, grows to 300 pt
  before it scrolls (macOS showed two accounts), names rows by alias or email local part cut in the
  middle, shows plan labels only when plans differ, and no longer dims an account at its limit. The
  footer Switch goes to the account with most headroom rather than repeating the first row's Use.
  Force Switch in the row menu is now "Switch Without Saving Current Login…" and asks first.
  Auto-switch in the header is a switch; Windows can now turn it on and off from the flyout. macOS:
  VoiceOver can open a row's account menu and Use it; the status lines share one mini glass button
  style; a Use that cannot be used is grey; the wake note says "wake skipped: not switchable".

### Fixed

- The landing site no longer fails to deploy after a release: Pages deploys only from `main`, and the
  release-tag run was refused by the github-pages environment. Publishing a release now starts the
  site build on `main`.

- macOS: in a narrow window the Usage pane was wider than the window, pushing the sidebar off its
  left edge. Each account's week now stacks (name and %, the tide, then pace and reset) when a row
  no longer fits, and the history controls wrap onto two lines, as the Windows app already did.
- macOS: purging one unclaimed credential now asks first, as on Windows.
- macOS: a collapsible list (profile categories, export accounts, an account's Advanced settings,
  the login's terminal output) opened only from its small chevron; a click anywhere on its row now
  opens or closes it, and its rows line up with the label instead of being indented. Windows export
  checkboxes now come before the account name, as on macOS.
- Both apps: "Also read Fable's weekly limit" sat above the setting it says it needs; it now
  follows "Read setup-token usage" and is off while that setting is off.
- Both apps: a hook whose command ends in a flag value (`--node /path/node`) was summarised as the
  flag's program ("node"); it now names the script the hook runs.

## [0.2.0] - 2026-11-27

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
- Usage share between machines (`usage.shareBetweenMachines`, on by default; acts while signed in
  to HaryX with sync unlocked): your machines spend one usage-endpoint budget per Claude account
  instead of one each. Each account gets an encrypted `usage` sync row with the newest reading, the
  shared poll plan, the failure state and a request ledger; a machine about to fetch first reads
  it, adopts a fresher reading, takes a lease so only one machine fetches, and writes its result
  back. A 429 on one machine parks the others for the same deadline, an account active on any
  machine keeps the active cadence everywhere, and every account is capped at 24 fetches an hour
  across machines. Without the share (signed out, offline), a machine that saw others poll an
  account in the last hour polls it that much less often. `list --json` marks such readings with
  `usageFromOtherMachine`; the macOS and Windows apps say "on another machine" next to the reading's
  age. Needs the server migration `20261127000000_sync_rows_usage.sql`.
- `haryx sync profile push|pull|list|remove`: setup profiles on the server, encrypted (secrets
  redacted, no history; three at most).
- `haryx account delete`: deletes the HaryX account and everything synced.
- macOS and Windows apps: a HaryX Account pane (sign in, unlock, sync, change passphrase, sign out,
  delete), automatic sync every 15 minutes and after a synced preference changes, and "On your other
  machines" in Accounts.
- macOS and Windows apps: HaryX sign-in lives in the dashboard sidebar's footer, a dropdown of the
  ways to sign in (GitHub or Google). Signed in, the footer shows the account and
  sync state, with Account & Sync and Sign Out in its menu.
- Account groups: `haryx group <num|email|alias> <name>` adds an account to a group, and an account
  can be in several (up to 16); `--remove <name>` takes it out of one, `--unset` out of all, and no
  account lists the groups. `haryx group --rename <old> <new>` renames a group on every account in
  it (a name of another group merges the two), and `haryx group --delete <name>` takes every
  account out of it; the accounts stay. Names are up to 32 characters and match ignoring case. Automatic
  switching (rotation, `best`, `next-available` and `haryx auto`) prefers accounts that share a
  group with the active account, ungrouped accounts counting as a group of their own, and leaves
  them only when none can take over, saying so in its warnings; `switch <num>` is unaffected. The
  groups show in `list`, and in `list --json` as `groups` (with `group`, the first, for older
  readers); they travel with export/import and sync. A machine on an older HaryX sees and edits only
  an account's first group; its change joins the front of the list and keeps the others. Continuing
  a stuck session on the best account (`haryx ai sessions continue --to best`, and the apps'
  Continue on…) prefers the session's groups the same way.
- macOS and Windows apps: groups. The Accounts pane shows a collapsible section per group, an
  account in several groups showing in each; Next in line and "Claude as…" are grouped. A New Group
  button (toolbar on macOS, footer on Windows) names a group and ticks its accounts in one go; a
  card's right-click menu and the menu bar / tray flyout account menu have a Groups menu (tick a
  group to add or remove it, New Group…, Remove from All Groups); and the details panel's Groups
  field lists the account's groups, each with a remove button, and adds one. A group's section
  header has Rename Group… and Delete Group… (its … button, and right-click on macOS); Delete asks
  first and says how many accounts it changes. Group changes sync to your other machines within a second or
  two (when sync is on), rather than at the next 15-minute sync. The Windows app gained
  a card right-click menu for this. From the tray flyout, New Group… opens the dashboard on that
  account rather than the dialog, as the flyout has no room for it.
- macOS and Windows apps: the Accounts pane filters by group (a group, or Ungrouped) and by status
  (On: in rotation, Off: held out of rotation). On macOS it is the toolbar's Filter menu; on
  Windows, the Group and Status pickers above the cards. While a filter is on, the pane says how
  many accounts it shows and offers Clear. Filters are per window and are not saved or synced.
- macOS and Windows apps: the account in use has its own wider card above the others, with a
  larger mascot and 5-hour figure and a cyan "In use" by its name; the grid below holds only the
  accounts you can switch to. An Off account (held out of rotation) steps back: a lighter card, a
  pause mark before its name, its usage as one line, and Turn On in place of Switch. Switch is now a
  secondary button, so cyan stays on the live account. On macOS the Set Up/Open Desktop button on
  the grid cards is an icon (its title in the tooltip), and Off accounts no longer show a wake note.
- macOS and Windows apps: a click on empty space in the Accounts pane closes the account's details
  panel, and the panel opens and closes with the cards moving alongside it instead of jumping
  (instant with Reduce Motion). Cards keep one width: the panel changes how many fit in a row, not
  how wide they are.

- Will the week last: `haryx list --json` gains `rotation`, a verdict over the accounts auto-switch may
  use (in rotation, not quarantined, subscription accounts, each up to its weekly switch line):
  their room left, the fleet's burn rate (measured over the last 72 h of kept readings, else each
  account's average since its reset), the next reset, and when the pool runs dry within the week.
  `haryx list` ends with the same verdict in one line.
- Apps (macOS and Windows alike), Usage: the page opens on that verdict ("The week lasts" or "Runs
  dry Sat 10:28") and one row per account with its Week Tide, weekly %, pace and reset; a row opens
  the account's details. It works with usage history off. The breakdown follows under "Where it
  went", where the range and API cost controls now apply.
- Apps, account cards: the weekly bar is now the Week Tide, the week so far as a curve against the
  pace diagonal (stretches with no readings dashed). Cards are about 13 pt taller.

### Changed

- Apps, Usage charts: each account keeps one fixed colour in every chart and table, with deeper
  versions on light backgrounds. No account colour is a reading colour any more (yellow and lime
  left the palette), and the active account is no longer redrawn in cyan, which recoloured past
  days after every switch.
- Apps, account cards: long names are cut in the middle, so accounts on one domain stay told apart
  (`Dep1pj.claude02` vs `Dep1pj.claude03`).
- Apps, AI section (macOS and Windows alike):
  - **Sidebar:** the AI panes now sit under three headings in sentence case:
    - Monitor: Accounts, Usage, Sessions
    - Automate: Auto-switch, Wake, Folder mappings
    - Set up: Setup profiles, AI settings, Backup & maintenance

    Windows renames Tools to Backup & maintenance, and uses drawn icons instead of text symbols.
  - **Settings wording:** settings show plain labels with a caption; the `haryx config` key is in the
    hover help. For example, `autoswitch.hysteresisPct` reads "Headroom margin (pts)".
  - **Auto-switch rules:** these settings live in Auto-switch only, and are no longer repeated in AI
    settings. On Windows, the rules are now typed controls that save as you change them, in place
    of text fields with a Save button on each row.
  - **Usage charts:**
    - A line now breaks where readings are missing, instead of drawing a straight line across the
      gap.
    - Each account keeps one colour in every chart, the active account in cyan.
    - Axis figures read 400M / 1.2B.
    - When usage history is off, a button turns it on.
  - **Continue on…** asks before it ends a session. On Windows it is a menu instead of a list that
    acted as soon as you picked an account. Windows also asks before Unmap and Forget, and errors
    stay on screen until dismissed.
  - **Layout and saving:** settings panes keep their form at 720 pt, centred, and now scroll across
    the whole window. Before, on macOS, only the form column scrolled. Wake saves as you change it.
  - **Wake:**
    - "All accounts" shows a mixed state when only some accounts are on.
    - Wake times read as relative times ("Woke 2 hr ago · next in 34 min").
    - An Off account's wake skip reads "Skipped: account is off".
  - **Backup & maintenance:**
    - "Replace accounts that already exist" replaces "--force".
    - The CLI section is titled "HaryX CLI".
    - Purge moves to its own "Erase" section at the end.
  - **Windows catches up with macOS:**
    - Shell picker, "Test: open a new shell" and "Send test notification"
    - Export picks several accounts
    - Switch and Refresh on Accounts
    - Captions at 12 px
- Apps: an Off account's card (held out of rotation) shows the same usage reading as other cards:
  the 5-hour figure with its reset, its staff and pace, and the week with its reset. Before, it
  showed only a one-line "5-hour N% · week N%". The card still steps back with a lighter fill, a
  pause mark, a dimmed mascot and Turn On. "Off: held out of rotation" now sits on the note line,
  so Off and On cards are the same height. The CLI already polled Off accounts and `haryx ai list`
  already showed their usage, so it is unchanged.
- Apps: every account card (Off ones included) tracks its weekly pace on a line under the week:
  "N pts ahead of pace · runs out Sat 10:15" in amber when the week is 5 or more points ahead (the
  run-out only when the CLI projects it will not last to the reset), else "N pts under pace" or
  "on pace". It reads the CLI's weekly `expectedPct`, so the line stays empty for the first day
  after a reset. The CLI already sent these fields; it is unchanged.
- Apps: an Off account's usage bars (on its card and in the details panel) are grey where they
  would be cyan, since cyan means live. Amber (ahead of pace, or 85% and over) and red (at the
  limit) stay, so an Off account that is being used elsewhere still stands out.
- macOS app: Use no longer quits and reopens a running Claude Desktop. Desktop opens on an account
  only from its Open/Set Up Desktop button or the panel's Desktop line. The setting is renamed
  "Claude Desktop per account", and the "Claude Desktop follows Use" roadmap item is dropped. macOS only: the Windows app has no Desktop launcher.
- Both apps: the account details panel is reorganised. A header shows the account's mascot, name,
  and state (in use, sessions running, out of rotation, sign-in needed), with Switch (⌘↩ on
  macOS), Sign In Again when it is due, and Run Claude. Then come Usage, Live Claude sessions
  (now on Windows too) and Switching (rotation, group, threshold). Mascot, alias, priority,
  position, swap and the forced switch, renamed "Switch Without Saving Current Login…", sit under a
  collapsed Advanced section. Settings save as soon as they change; text fields save on
  Return or when you leave them, and no longer lose a half-typed edit when the selection
  changes or the account list refreshes. The Windows panel has no per-account Wake toggle yet;
  Wake is set in its Wake pane.

### Removed

- Apps, Usage: the "Quota over time" chart; the week rows replace it.

### Fixed

- Both apps: the account details panel's "Weekly pace" no longer says "On track" for an account
  well ahead of pace (32% used against an expected 18%) while its week bar shows amber. It now uses
  the cards' wording and threshold: "14 pts ahead", "N pts under" or "On pace", with the expected
  percentage. The Windows panel gains the Weekly pace and Projected run-out lines macOS had.
- A running Claude session started with plain `claude` picks up a switched account by itself, so
  the apps no longer call it stuck after a switch, and macOS no longer notifies "N sessions stopped
  on account A" after an auto-switch. Only a session started with `haryx run` (tied to its account)
  is offered Continue on…. `haryx ai sessions continue` no longer restarts a session on the account
  it already runs as.

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

[Unreleased]: https://github.com/conghai20/haryx/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/conghai20/haryx/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/conghai20/haryx/compare/v0.1.6...v0.2.0
[0.1.6]: https://github.com/conghai20/haryx/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/conghai20/haryx/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/conghai20/haryx/compare/v0.1.2...v0.1.4
[0.1.2]: https://github.com/conghai20/haryx/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/conghai20/haryx/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/conghai20/haryx/releases/tag/v0.1.0
