# HaryX for Windows: tray app plan

Status: built (2026-11-24), all four phases, on branch `feat/windows-plan`. Tested by CI on
`windows-latest` (CLI tests, app backend tests, UI type check and tests) and by running the app on
macOS. **Not yet run on a real Windows machine**: the tray, toasts, terminals, the wake task and the
installer need the manual checklist below before a release.

Built as planned, with these differences:
- The app does not link `haryx-core`: it decodes the CLI's `--json` output with its own types, tested
  against `packages/contracts/`, as the macOS app does. Fewer couplings, same contract.
- Running sessions come from the new `haryx ai sessions --json`, which reads Claude Code's own
  `sessions/{pid}.json` records (already cross-platform), not from reading process environments. No
  process-memory reading was needed.
- Mascot sprites stay defined in the macOS app's `Sprites.swift`; `scripts/brand/export_sprites.py`
  writes `packages/mascots/sprites.json` for Windows and CI fails if the two drift. Moving the macOS app
  onto the JSON is left for later.
- No Mica backdrop: a transparent window shows through on Windows 10, so the dashboard uses flat fills.
- `haryx run` needs no Windows terminal code: the app opens the terminal and runs `haryx run N` in it.
- `haryxw` is built only with the `windowless` feature (Windows releases), so other platforms do
  not ship it. A `cargo install` on Windows has no `haryxw`; the wake task then runs `haryx.exe` and
  a console window flashes each minute.

Still open:
- **Updates inside the app**: built without `tauri-plugin-updater` and without a signing key (see
  `docs/in-app-update-plan.md`): the bundled haryx downloads the setup.exe and checks its SHA-256,
  and the app runs it silently after quitting. Needs the real-hardware check below.
- **Code signing**: unsigned by decision for now.
- **winget**: the release renders the manifests; submitting them to `microsoft/winget-pkgs` is a
  manual pull request.
- **Claude Desktop follow** on Windows still needs its sign-in test; the app does not offer it.
- **Unclaimed credentials** are read from `haryx unclaimed` text (no `--json` yet), as on macOS.
- **Folder mappings pane**: macOS lists mapped folders (with each account's mascot) in Tools;
  Windows has no such pane yet.

## Goal

A Windows tray app that does for Windows what `apps/macos/` does for macOS: one click to see every Claude
account's usage and to switch the active one, plus auto-switch, sessions, terminals, notifications, wake and
profiles. It is a GUI over the `haryx` CLI, so the CLI must work on Windows first (Phase 1).

## Decisions

| Topic | Decision |
|---|---|
| Stack | Tauri 2: Rust backend, web UI in the system WebView2. |
| Talking to haryx | Run the bundled `haryx.exe` (Tauri sidecar) with `--json`, the same contract the macOS app uses. Link `haryx-core` only for its serde types, so the UI decodes exactly what the CLI prints. |
| Who owns state | The CLI. The app never touches credentials or the store directly. Same rule as macOS. |
| Location | `apps/windows/`. The Tauri code is not Windows-only, so it could become the Linux app later; the folder is renamed if that happens. |
| Min OS | Windows 10 22H2 and Windows 11, x64 and arm64. |
| UI | Svelte 5 + TypeScript + Vite. |
| Signing | Unsigned at first; signing later. |
| WSL | Out of scope for v1. |
| Look | The HaryX brand from `DESIGN.md` (colours, Space Grotesk), adjusted to Fluent conventions: Segoe UI Variable for body text, Mica backdrop on the dashboard, a tray flyout in place of the menu bar popover. |

Why sidecar over linking `haryx-core` in process: the `--json` shapes are already pinned in
`packages/contracts/` and tested by the macOS app; a CLI crash cannot take down the tray; the app and the
CLI on `PATH` cannot drift apart because the app runs its own copy. One `list --json` costs about 0.25 s,
which is fine at the polling rates below.

## Phase 1: haryx CLI on Windows (prerequisite)

Each item replaces a `TODO(windows)` stub that `PORTING.md` left. Done when `cargo test` passes on a
`windows-latest` CI job and the commands below work on a real machine.

| Area | Today | Windows design |
|---|---|---|
| Store root | `paths.rs` stub | `%LOCALAPPDATA%\haryx` (not roaming: it holds per-machine credentials). Honour `HARYX_HOME`. Claude Code's own folder stays `%USERPROFILE%\.claude`. |
| Live credentials | Keychain on macOS, `.credentials.json` elsewhere | Claude Code on Windows stores its login in `%USERPROFILE%\.claude\.credentials.json`, so the existing file path applies. Check this against the current Claude Code release before coding. |
| Saved account credentials | Keychain / files | Files under the store, encrypted with DPAPI (`CryptProtectData`, current user). Credential Manager is rejected: its 2560-byte blob limit is too close to an OAuth JSON with scopes and metadata. |
| Process alive / start time | `ps`, `/proc` | `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + `GetProcessTimes`, via `windows-sys`. |
| Running sessions | libproc + `KERN_PROCARGS2` (macOS app) | Read `CLAUDE_CONFIG_DIR` from each candidate's environment block: `NtQueryInformationProcess` gives the PEB, `ReadProcessMemory` reads `ProcessParameters.Environment`. Same-user processes only. Filter on image name first (`claude.exe`, or `node.exe` whose command line runs Claude Code). Exposed as a new `haryx sessions --json` so both apps share one scanner. |
| Links | symlinks | Directory junctions (no admin); file symlinks only when Developer Mode allows, else copy. Strip `\\?\` prefixes. |
| Lock mtime | `utime` | `SetFileTime`. |
| Console colour | ANSI | Turn on VT processing at start (`SetConsoleMode`). |
| Terminal launch (`run`) | macOS terminals | Windows Terminal (`wt.exe new-tab`), PowerShell 7 (`pwsh`), Windows PowerShell, cmd, Git Bash. Pass `CLAUDE_CONFIG_DIR` through the child environment, never through a quoted command string. |
| Wake runner | user LaunchAgent | Per-user Task Scheduler task `HaryX\Wake`, every minute, "run as soon as possible after a missed start" (the Windows form of "fire late after sleep"). Installed with the Task Scheduler COM API or `schtasks /Create /XML`. No admin. |
| Claude Desktop follow | `--user-data-dir` per account | Needs its own Phase 0 test on Windows: Desktop installs through Squirrel/MSIX, and the launch path and whether `--user-data-dir` is honoured both need checking. Until then the Desktop line shows "not supported on Windows yet". |
| Packaging | macOS/Linux binaries | Add `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc` to the release workflow, the npm/pip wrappers and a winget manifest. |

New contracts before code: `packages/contracts/haryx/sessions.json` and a Windows `list` fixture with
Windows paths.

## Phase 2: app shell

- `apps/windows/` Tauri 2 project: `src-tauri/` (Rust) and `ui/` (web).
- UI framework: Svelte 5 + TypeScript + Vite. Small output, no virtual DOM, close to the zero-dependency
  habit of the macOS app.
- Rust backend modules mirror `HaryXCore`: `cli` (run the sidecar, decode JSON, answer y/N on stdin after
  the app confirms), `poll` (list every 60 s, on flyout open, and on a file watch of `sequence.json`),
  `sessions` (every 4 s), `notify`, `terminals`, `tray`.
- Tray icon: left click opens the flyout, right click opens a short menu (Use next, Open dashboard, Quit).
- Flyout: frameless, anchored above the tray, closes on blur. Account cards with 5 h and 7 d bars,
  the Use button, and quick actions.
- Dashboard window: the same panes as the macOS dashboard (Accounts, Auto-switch, Wake, Profiles, Tools,
  Settings).

## Phase 3: features, in order

1. List, usage, active account, Use (switch), next / best / next-available.
2. Add account (login flow in a terminal, as on macOS), remove, enable/disable, alias, priority, move, swap.
3. Auto-switch: long-running `haryx auto --json` child with a live log.
4. Running sessions and "Claude in folder" quick actions; mapped folders.
5. Notifications: Windows toasts through `tauri-plugin-notification` with the same rules as
   `NotificationRules.swift` (port the rules, keep the thresholds).
6. Wake pane (status, opt-in, quiet hours, install/remove the scheduled task).
7. Profiles pane.
8. Mascots in the tray: the sprite data moves from `Sprites.swift` into a shared JSON file in
   `packages/`, so both apps read one source. The Windows app renders 16/20/24/32 px frames for the tray
   DPI and swaps icons at 4 fps idle / 8 fps running, static when "Show animations" is off in Windows
   settings.
9. Launch at login (`tauri-plugin-autostart`, HKCU Run key).

## Phase 4: distribution

- NSIS installer (per user, no admin) built by the Tauri bundler; WebView2 bootstrapper for Windows 10.
- Unsigned at first. SmartScreen warns on a browser download (the Windows form of the Gatekeeper problem
  fixed in 0.1.2), so winget and the PowerShell installer are the main install routes. Authenticode signing
  (Azure Trusted Signing or a certificate) comes later.
- Updates through `tauri-plugin-updater` with a signed manifest on GitHub releases.
- winget manifest; a PowerShell one-liner installer like the macOS `curl … | sh`.

## Testing

- Rust unit tests for every new `cfg(windows)` path, run on a `windows-latest` CI job.
- App backend: contract fixture decoding (as `HaryXCoreTests` does), notification rules, terminal command
  building, tray frame selection.
- UI: component tests with Vitest; one Playwright-on-WebView2 smoke test (open flyout, press Use).
- Manual checklist on a real Windows 11 machine for each release: sign in, switch, session detection
  with Claude Code running, toast, wake task firing after sleep.

## Out of scope for now

- Claude Code running inside WSL. It has its own Linux `~/.claude`; a later phase could talk to a
  `haryx` inside the distro.
- Microsoft Store packaging (MSIX). Its sandbox would block reading other processes' environments.

## Open questions

- Whether the Tauri app should also replace the macOS app later. Proposed: no; macOS stays native SwiftUI.
