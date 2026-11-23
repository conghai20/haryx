# Architecture

## Repository layout

| Path | Part | Status |
|---|---|---|
| `apps/macos/` | HaryX macOS app (SwiftUI menu bar + dashboard) | shipping |
| `cli/` | `haryx` Rust CLI, drop-in replacement for `cswap`; owns accounts and credentials on the device | in progress |
| `server/` | HaryX server: sync, team view, API for web and mobile (Go) | future |
| `apps/windows/` | HaryX Windows tray app (Tauri 2), plan in `docs/windows-app-plan.md` | planned |
| `apps/web/` | web dashboard | future |
| `apps/ios/`, `apps/android/` | native mobile apps | future |
| `packages/contracts/` | language-neutral JSON fixtures/schemas shared by every part | active |
| `packages/design-tokens/` | palette, type, spacing generated per platform | planned (after the palette is final) |
| `assets/brand/`, `scripts/brand/` | logo sources and generators (move to `packages/brand/` once logo work settles) | active |
| `upstream/claude-swap/` | local, unpushed copy of upstream claude-swap (MIT) used as reference for the Rust port | reference |
| `packaging/` | Homebrew, crates.io, npm, PyPI, signed DMG | planned |

Folders for future parts are created only when that work starts.

### Rules
- **Credentials stay on the device.** The CLI owns accounts and tokens. The server only ever receives metadata
  (usage, session status). Web and mobile can display and request; they never hold credentials.
- **Contracts first.** A new data shape lands in `packages/contracts/` before code uses it.
- **One design-token source** generated for every platform.

## macOS app

Native macOS (SwiftUI, macOS 26+, Liquid Glass) GUI over `cswap` (claude-swap ≥ 1.1), later `haryx`.
Zero third-party dependencies. SwiftPM build → ad-hoc signed `.app`.

## How it talks to cswap
| Need | Source |
|---|---|
| accounts, usage, plan, active | `cswap list --json` (poll, default 60 s, + on popover open, + FS watch of `sequence.json`) |
| settings | `cswap config list --json`, `config set/unset` |
| priorities / unclaimed | parse `cswap priority` / `cswap unclaimed` text |
| directory mappings | read `~/.claude-swap-backup/mappings.json`, write via `cswap map/unmap` |
| auto-switch | child process `cswap auto --json` (one JSON event per line) |
| y/N prompts (remove, purge, overwrite slot) | app confirms first, then answers on stdin |
| running claude sessions | native `proc_listallpids` + `KERN_PROCARGS2` → `CLAUDE_CONFIG_DIR` → slot |

## macOS app layout (`apps/macos/`)
- `Sources/HaryXCore` — pure, tested: models/JSON, CLI runner, process scanner, notification rules, sprites + CG pixel renderer, shell quoting.
- `Sources/HaryX` — SwiftUI app, grouped by feature: `App/` (entry, AppStore), `MenuBar/`, `Dashboard/`,
  `Preferences/`, `Mascot/`, `Services/` (auto-switch runner, notifier, terminal launcher), `DesignSystem/`.
- `Tests/HaryXCoreTests` — contract fixture decoding, slot mapping, rules, sprite integrity, quoting.
- `scripts/build.sh` — release build, bundle, brand icon, codesign, install.

## Features (all cswap commands)
list/status · switch (target / next / best / next-available / force) · add (slot, alias, overwrite) · add-token (stdin) ·
remove · enable/disable · alias · threshold · priority · move · swap · map/unmap · run (in a terminal) · auto (+ live log) ·
config (all keys) · export (account, --full) · import (--force) · import-usage · unclaimed (+purge) · upgrade · purge.

## Extras
- Menu bar: active mascot + 5h %, animated while Claude runs; glass popover with all accounts and quick actions.
- Pixel mascots (8 original species), per-account species/colour; states: idle (breathe+blink), running (walk),
  tired (≥90 % usage), sleeping (disabled), error (re-login). Reduce Motion → static.
- Terminals: Terminal, iTerm2, Warp, Ghostty, kitty, WezTerm, Alacritty (installed ones only) + shell choice.
  Quick actions: new shell, Claude as account N, Claude in folder…, mapped folders.
- Notifications: switched, usage 80/95 %, projected exhaustion, re-login/expiry, quota reset,
  Claude session finished (≥60 s), auto-switch quarantine/all-exhausted/error.
- Launch at login (SMAppService).

## Performance
- One `list --json` ≈ 0.25 s; refreshes coalesced; never on main thread.
- Mascot frames are tiny cached CGImages; TimelineView only while visible; 4 fps idle / 8 fps running.
- Process scan every 4 s, path-filtered before reading argv/env (~ms).
