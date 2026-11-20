# cswapui — plan

Native macOS (SwiftUI, macOS 26+, Liquid Glass) GUI for `cswap` (claude-swap ≥ 1.1).
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

## Layout
- `Sources/CSwapCore` — pure, tested: models/JSON, CLI runner, process scanner, notification rules, sprites + CG pixel renderer, shell quoting.
- `Sources/cswapui` — SwiftUI app: menu bar panel, dashboard window, terminal launcher, notifier, auto-switch runner.
- `Tests/CSwapCoreTests` — decoding fixture, slot mapping, rules, sprite integrity, quoting.
- `scripts/build.sh` — release build, bundle, icon (rendered by the app itself: `cswapui --render-icon`), codesign, install.

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
