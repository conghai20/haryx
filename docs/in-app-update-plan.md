# In-app updates (macOS, Windows, CLI): plan

Status: built (2026-11-24), branch `feat/in-app-update`. CLI, macOS app and Windows app are tested
locally (CLI and Windows backend on macOS; the macOS updater against a real disk image). The Windows
install path (silent NSIS run and relaunch) still needs a real Windows machine.

Built as planned, with these differences:
- The Windows app does not download itself: `haryx upgrade --download-app DIR --json` (the bundled
  haryx) fetches and checks the setup.exe, so the checksum code exists once, in `haryx-core`.
  The macOS app downloads in Swift (CryptoKit), since it also copies the app out of the DMG.
- `haryx upgrade` from inside the app only says the app updates it; it does not open the app.
- The app compares the release with its own version (`CFBundleShortVersionString`, the Tauri
  crate's version), not the CLI's, so a user's own older haryx cannot hide an app update.
- The passive "a newer version is available" notice after CLI commands stays unwired: it would add a
  network call to every command.
- External links in the Windows app go through an allowlisted `open_link` (HaryX's GitHub pages
  only), since the webview has no opener plugin.
- The menu bar row lives in `MenuBar/UpdateLine.swift`; the one line that shows it in
  `MenuBarPanel.swift` is left uncommitted with the owner's other edits to that file.

Decisions taken with the owner on 2026-11-24:
- macOS: our own updater, reusing the `install.sh` steps. No Sparkle.
- Windows: our own updater, reusing the `install.ps1` steps. No `tauri-plugin-updater`, no key.
- How much the apps do on their own is a setting, in a new **About & Updates** section that also
  shows the app version and release notes.
- Standalone CLI installs (tarball, install script) replace themselves through `haryx upgrade`.

## What exists

- Every release carries `HaryX-<v>.dmg`, `HaryX-<v>-{x64,arm64}-setup.exe`,
  `haryx-v<v>-<target>.tar.gz`, each with a `.sha256`. The release is created as a draft and
  published by hand, so GitHub's `releases/latest` only ever names a published release.
- `scripts/install.sh` (tested by `scripts/test-install.sh`) and `scripts/install.ps1` already
  download, verify and swap the app.
- Both apps bundle the CLI. macOS links `haryx` into `HaryX.app` (`CLIInstaller.swift`); Windows
  runs it from the install folder's `bin/`. Updating the app updates that CLI.
- `haryx-core/src/update_check.rs`: cached fetch and version compare, switched off
  (`RELEASES_URL = None`). `haryx upgrade` handles Homebrew, cargo, pipx, uv and npm only.

## Trust model

The `.sha256` comes from the same release as the file, so it proves the download is intact, not
who made it: anyone who can publish a release on `conghai20/haryx` can ship an update. That is the
same trust `install.sh`/`install.ps1` ask for today. Signed updates (Sparkle EdDSA, the Tauri
updater key) can be added later without changing the UI.

## Phase 1: the check, in the CLI

One check both apps share, so the release lookup and asset choice exist once.

- `RELEASES_URL = Some("https://api.github.com/repos/conghai20/haryx/releases/latest")`. Existing
  cache rules stay (a day on success, an hour on failure). `updates.check = false` still turns
  the passive notice off.
- `haryx upgrade --check [--json] [--refresh]`: no install, prints
  `{current, latest, available, published_at, notes, html_url, assets: {app, cli}}`, where `app`
  and `cli` are `{name, url, sha256_url, size}` for this OS and architecture (`null` if none).
  `notes` is the release body (rendered from CHANGELOG.md by `scripts/release/notes.sh`).
  `--refresh` skips the cache ("Check now" button).
- Contract: `packages/contracts/upgrade-check.json` plus a fixture, decoded by both apps' tests.
- Tests: asset choice per target, draft/pre-release ignored, `v` prefix stripped, cache hit and
  miss, network failure gives `latest: null` without an error exit.

## Phase 2: CLI install detection and self-replace

`detect_install_method` gains:
- **App-bundled**: the executable resolves inside `*.app/Contents/Resources/bin/` or the Windows
  install folder's `bin/`. `haryx upgrade` says "this haryx is updated by the HaryX app" and, on
  macOS, opens the app's update section (`open -b dev.haryx.app --args --show-updates`). Exit 0.
- **Standalone** (no other method matched): download the tarball for this target, check the
  sha256, unpack to a temp folder next to the binary, check the new binary runs
  `--version` and reports the new version, then replace it atomically (rename over). On Windows the
  running `.exe` cannot be overwritten: rename it to `haryx.exe.old`, move the new one in, delete
  `.old` on the next start. `haryxw.exe` is replaced alongside when present.
- No write access to the folder: say so with the path; nothing changed.
- The passive notice names the right action per method (`brew upgrade haryx`, "update HaryX",
  `haryx upgrade`).
- Tests: detection per path shape; self-replace against a `file://` release (same layout as
  `test-install.sh`), bad checksum refused, read-only folder refused, Windows rename path behind
  `cfg(windows)` and run in `cli-windows` CI.

## Phase 3: macOS app

**About & Updates** section (dashboard group "HaryX", after App Preferences):
- About: app icon, version and build, bundled CLI version, CLI install state, links (GitHub,
  releases, licence).
- Updates: mode picker, **Automatic** (download in the background, install on next quit or when
  the user clicks "Restart now"), **Notify** (default: check daily, badge and "Update to vX"),
  **Manual** (check only on "Check now"). Last checked time. "Check now".
- Release notes: the latest release's notes (Markdown rendered with `AttributedString`), and the
  installed version's notes from the bundled CHANGELOG.md when no update is waiting.
- Menu bar panel: a small "Update to vX" row when one is available (in Notify and Automatic mode).

Install (`HaryXCore/AppUpdater.swift`, a port of `install.sh`'s steps, testable without UI):
1. Download the DMG to `~/Library/Caches/dev.haryx.app/updates/`, check its sha256.
2. `hdiutil attach -nobrowse -readonly`, `ditto` the app beside the current one as
   `HaryX.app.new`, detach.
3. `codesign --verify --strict` the new app; remove `com.apple.quarantine`.
4. Swap: move the current app aside, move the new one in, delete the old one. A small detached
   shell script does the swap after HaryX quits, then `open`s the new app.
5. The app is in a folder it cannot write (for example `/Applications` owned by another user) or
   is translocated: open the DMG in Finder and say why instead of failing quietly.

Prefs: `updateMode` (`automatic | notify | manual`, default `notify`), `lastUpdateCheck`,
`skippedVersion` ("Skip this version"). App updates do not change the CLI's `updates.check`.

Tests: `AppUpdaterTests` for the steps against a fake DMG (as `test-install.sh` builds one),
checksum failure, unwritable target; contract decode of `upgrade --check --json`.

## Phase 4: Windows app

Same section in Svelte (`panes/AboutPane.svelte`), same three modes, settings in the app's
`settings.rs`.

Install (`src-tauri/src/updater.rs`, a port of `install.ps1`):
1. Download the setup.exe for this architecture to `%LOCALAPPDATA%\HaryX\updates\`, check its sha256.
2. Spawn detached `cmd /c "<setup.exe> /S && start "" "<install dir>\HaryX.exe"`", then quit the
   app so NSIS can replace files (per-user install: no UAC prompt).
3. Automatic mode installs on the next app quit, or on "Restart now".

Tests: Rust unit tests for asset choice and the command line built; Vitest for the pane;
real-hardware checklist entry (silent install, relaunch, tray comes back, wake task still runs).

Windows real-hardware checklist for updates:
- [ ] About & updates shows the version and the latest notes; Check now works.
- [ ] Update downloads into `%LOCALAPPDATA%\dev.haryx.app\updates` and shows "ready".
- [ ] Restart now: HaryX closes, the installer runs with no window or prompt, HaryX reopens on the
      new version, the tray icon comes back, the installer file is gone.
- [ ] Quit with an update ready installs it without reopening.
- [ ] The wake task and autostart still work after the update.

## Phase 5: release workflow and docs

- No new release assets are needed. Check that the release body is the CHANGELOG notes (it is).
- `CHANGELOG.md`, README (updating section), `docs/windows-app-plan.md` "Still open" updated.

## Order and size

1 then 2 (CLI, small), then 3 and 4 (can run side by side), then 5. One PR per phase or one
branch `feat/in-app-update` with a commit per phase.

## Open risks

- macOS: an ad-hoc signed app that replaces itself may lose permissions the user granted
  (Accessibility, Automation), since the signature changes each build. Developer ID builds keep
  them. To note in About when the build is ad-hoc.
- GitHub API rate limit (60 requests an hour without a token): the daily cache keeps us far under.
- Windows SmartScreen does not prompt for a silent install started by the app, but an unsigned
  installer can still be blocked by Defender policy on managed machines; fall back to opening the
  releases page.
