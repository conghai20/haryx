<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/haryx-lockup.svg">
    <img src="assets/brand/haryx-lockup-light.svg" alt="HARYX" height="72">
  </picture>
</p>

# HaryX

**Take control of your AI workflow and other DevSecOps, SRE, Cloud… all in one.**
A unified control center for AI coding accounts, usage and sessions.

Today HaryX is a native macOS menu bar app for Claude accounts managed by
[claude-swap](https://github.com/realiti4/claude-swap) (`cswap`). A Rust `haryx` CLI that replaces `cswap` (in progress, `cli/`),
a server, a web dashboard and native mobile apps are on the [roadmap](docs/roadmap.md).

## Repository

| Path | What |
|---|---|
| [`apps/macos/`](apps/macos) | HaryX macOS app (SwiftUI, macOS 26+) |
| [`packages/contracts/`](packages/contracts) | JSON contracts shared by the app and the CLI |
| `assets/brand/`, `scripts/brand/` | logo sources, exports and generators |
| [`upstream/`](upstream) | local reference copy of claude-swap (MIT) for the Rust port |
| [`docs/`](docs) | [architecture](docs/architecture.md), [roadmap](docs/roadmap.md), brand notes |

## macOS app

Requirements: macOS 26+, Xcode 26+ toolchain, `cswap` ≥ 1.1 (`uv tool install claude-swap`).

```sh
cd apps/macos
swift test                  # core tests
scripts/build.sh            # → apps/macos/build/HaryX.app
scripts/build.sh --install  # → ~/Applications/HaryX.app, launched
```

### What it does
- **Menu bar**: active account's mascot plus its 5-hour %. The mascot walks while a Claude session is using that account.
  Click it to open a glass panel with all accounts, usage bars, Switch (best / next / next-available), Auto-switch, and
  terminal quick actions.
- **Dashboard**: account cards plus an inspector with every cswap control: switch / force switch, alias,
  enable/disable, threshold, priority, move, swap, remove, run Claude as the account, live sessions, mascot species
  and colour. Also panes for auto-switch (live event log), folder mappings, every `cswap config` key,
  export/import/import-usage, unclaimed credentials, upgrade and purge.
- **Mascot moods**: idle (breathe + blink) · running (walk) · tired (≥90% quota, sweat) · sleeping (disabled) · error (re-login needed).
- **Terminals**: Terminal, iTerm2, Warp, Ghostty, kitty, WezTerm, Alacritty (installed ones are listed), plus shell choice.
  "Run Claude as account N" uses `cswap run N` (session mode, that terminal only).
- **Notifications**: account switched, usage 80/95% or projected run-out, re-login / expiring login / expired plan,
  quota reset, Claude session finished (≥1 min), auto-switch quarantine / all exhausted / errors. Each can be toggled.

### How it works
| | |
|---|---|
| Data | `cswap list --json`, polled (default 60 s), when the panel opens, and when `~/.claude-swap-backup/sequence.json` changes |
| Running sessions | native process scan (libproc + `KERN_PROCARGS2`): a `claude` process's `CLAUDE_CONFIG_DIR` gives its slot |
| Auto-switch | `cswap auto --json` as a child process; stopped on quit |
| Prompts | the app confirms first, then answers cswap's y/N on stdin; tokens go via stdin (`add-token -`) |

### Performance
CPU is ~0.3% idle and ~4% while the menu bar mascot walks; that cost comes from AppKit re-snapshotting the status
item each frame. Turn it off under Preferences → "Animate menu bar mascot". In-app mascots animate only while
visible (4 fps idle, 8 fps running) from cached frames.
