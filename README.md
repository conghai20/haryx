# cswapui

Native macOS menu bar app for [claude-swap](https://github.com/realiti4/claude-swap) (`cswap`), with Liquid Glass UI
and an animated pixel mascot for every Claude account.

## Requirements
- macOS 26+ (Liquid Glass), Xcode 26+ toolchain
- `cswap` ≥ 1.1 (`uv tool install claude-swap`)

## Build & run
```sh
swift test                  # core tests
scripts/build.sh            # → build/cswapui.app
scripts/build.sh --install  # → ~/Applications/cswapui.app, launched
```

## What it does
- **Menu bar**: active account's mascot plus its 5-hour %. The mascot walks while a Claude session is using that account.
  Click it to open a glass panel with all accounts, usage bars, Switch (best / next / next-available), Auto-switch, and
  terminal quick actions.
- **Dashboard** (panel → Dashboard, or re-open the app): account cards plus an inspector with every cswap control:
  switch / force switch, alias, enable/disable, threshold, priority, move, swap, remove, run Claude as the account,
  live sessions, mascot species and colour. Also panes for auto-switch (live event log), folder mappings, every
  `cswap config` key, export/import/import-usage, unclaimed credentials, upgrade and purge.
- **Mascot moods**: idle (breathe + blink) · running (walk) · tired (≥90% quota, sweat) · sleeping (disabled) · error (re-login needed).
- **Terminals**: Terminal, iTerm2, Warp, Ghostty, kitty, WezTerm, Alacritty (installed ones are listed), plus shell choice.
  "Run Claude as account N" uses `cswap run N` (session mode, that terminal only).
- **Notifications**: account switched, usage 80/95% or projected run-out, re-login / expiring login / expired plan,
  quota reset, Claude session finished (≥1 min), auto-switch quarantine / all exhausted / errors. Each can be toggled.

## How it works
| | |
|---|---|
| Data | `cswap list --json`, polled (default 60 s), when the panel opens, and when `~/.claude-swap-backup/sequence.json` changes |
| Running sessions | native process scan (libproc + `KERN_PROCARGS2`): a `claude` process's `CLAUDE_CONFIG_DIR` gives its slot |
| Auto-switch | `cswap auto --json` as a child process; stopped on quit |
| Prompts | the app confirms first, then answers cswap's y/N on stdin; tokens go via stdin (`add-token -`) |

## Performance
About 20 MB RSS. CPU is ~0.3% idle and ~4% while the menu bar mascot walks; that cost comes from AppKit
re-snapshotting the status item each frame. Turn it off under Preferences → "Animate menu bar mascot".
In-app mascots animate only while visible (4 fps idle, 8 fps running) from cached frames.

## Layout
- `Sources/CSwapCore`: models, CLI runner, process scanner, notification rules, sprites, pixel renderer, icon (tested)
- `Sources/cswapui`: status bar, SwiftUI views, terminal launcher, notifier, auto-switch runner
- `scripts/build.sh`: bundle + icon (drawn by the binary itself: `cswapui --render-icon out.png`) + ad-hoc signing
