<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/haryx-lockup.svg">
    <img src="assets/brand/haryx-lockup-light.svg" alt="HARYX" height="72">
  </picture>
</p>

# HaryX

**Take control of your AI workflow and other DevSecOps, SRE, Cloud… all in one.**
A unified control center for AI coding accounts, usage and sessions.

HaryX helps one person who keeps several of their own Claude subscriptions manage them in one place:
see each account's 5-hour and weekly usage, choose which account Claude Code is logged in as, and run
Claude as a given account per folder or terminal. It ships as:

- **`haryx`**, a command-line tool in Rust (`cli/`): a drop-in port of
  [claude-swap](https://github.com/realiti4/claude-swap) (`cswap`) with the same commands, flags and
  `--json` output, plus an interactive dashboard (`haryx ai tui`).
- **HaryX for macOS**, a native menu bar app (`apps/macos/`) that drives `haryx` (or `cswap`).

A server, a web dashboard and native mobile apps are on the [roadmap](docs/roadmap.md).

## Use within Anthropic's terms

HaryX is an independent project. It is not made, endorsed or supported by Anthropic.

- **Your own accounts only.** Use HaryX with subscriptions that you hold and pay for yourself. Never share
  a login, and never let other people use your accounts through it. You are responsible for all activity
  on your accounts.
- **Claude Code does the work.** HaryX sends no model requests. It stores each account's Claude Code login
  on your machine, reads each account's usage, and chooses which login the official, unmodified Claude Code
  uses.
- **Plan limits still apply.** Anthropic sets each plan's usage limits for ordinary, individual use, and its
  terms prohibit tools that route traffic against subscription limits. Auto-switch is off unless you start
  it; it moves work between your own plans when the active one reaches the threshold you set. Don't use it
  to get more out of a plan than the plan allows.
- **Anthropic decides.** Anthropic may warn, throttle, suspend or close accounts it believes break its
  terms. Read the [Consumer Terms](https://www.anthropic.com/legal/consumer-terms), the
  [Usage Policy](https://www.anthropic.com/legal/aup) and Claude Code's
  [legal and compliance](https://code.claude.com/docs/en/legal-and-compliance) page before you use HaryX.

## Install

```sh
brew install conghai20/tap/haryx     # macOS, Linux
cargo install --locked haryx         # from source
pipx install haryx                   # or: uv tool install haryx
npm install --global haryx
```

The macOS app's DMG bundles the same `haryx`. Publishing is set up but not yet live; see
[docs/release.md](docs/release.md). Until then, build from source (below).

## Repository

| Path | What |
|---|---|
| [`cli/`](cli) | `haryx` CLI and dashboard (Rust workspace: `haryx-core`, `haryx`) |
| [`apps/macos/`](apps/macos) | HaryX macOS app (SwiftUI, macOS 26+) |
| [`packaging/`](packaging), `scripts/release/` | PyPI and npm wrappers, release tarballs, Homebrew formula |
| [`packages/contracts/`](packages/contracts) | JSON contracts shared by the app and the CLI |
| `assets/brand/`, `scripts/brand/` | logo sources, exports and generators |
| [`upstream/`](upstream) | local reference copy of claude-swap (MIT) for the Rust port |
| [`docs/`](docs) | [architecture](docs/architecture.md), [roadmap](docs/roadmap.md), brand notes |

## CLI

```sh
cd cli
cargo test --workspace      # tests
cargo run -p haryx -- list  # run from source
cargo install --path crates/haryx   # → ~/.cargo/bin/haryx
```

See [cli/README.md](cli/README.md) for the commands. haryx keeps its own data folder
(`~/Library/Application Support/HaryX`, `$XDG_DATA_HOME/haryx` on Linux, or `HARYX_HOME`);
`haryx ai import --from-cswap` copies an existing cswap setup and leaves cswap untouched.

## macOS app

Requirements: macOS 26+, Xcode 26+ toolchain, and `haryx` (or `cswap` ≥ 1.1) on one of the usual paths.
The app uses `haryx` when it finds one and falls back to `cswap`.

```sh
cd apps/macos
swift test                  # core tests
scripts/build.sh            # → apps/macos/build/HaryX.app
scripts/build.sh --install  # → ~/Applications/HaryX.app, launched
scripts/package-dmg.sh      # → apps/macos/build/HaryX-<version>.dmg with haryx bundled
```

### What it does
- **Menu bar**: active account's mascot plus its 5-hour %. The mascot walks while a Claude session is using that account.
  Click it to open a glass panel with all accounts, usage bars, Switch (best / next / next-available), Auto-switch, and
  terminal quick actions.
- **Dashboard**: account cards plus an inspector with every CLI control: switch / force switch, alias,
  enable/disable, threshold, priority, move, swap, remove, run Claude as the account, live sessions, mascot species
  and colour. Also panes for auto-switch (live event log), folder mappings, every `config` key,
  export/import/import-usage, unclaimed credentials, upgrade and purge.
- **Mascot moods**: idle (breathe + blink) · running (walk) · tired (≥90% quota, sweat) · sleeping (disabled) · error (re-login needed).
- **Terminals**: Terminal, iTerm2, Warp, Ghostty, kitty, WezTerm, Alacritty (installed ones are listed), plus shell choice.
  "Run Claude as account N" uses `haryx run N` (session mode, that terminal only).
- **Notifications**: account switched, usage 80/95% or projected run-out, re-login / expiring login / expired plan,
  quota reset, Claude session finished (≥1 min), auto-switch quarantine / all exhausted / errors. Each can be toggled.

### How it works
| | |
|---|---|
| Data | `haryx list --json`, polled (default 60 s), when the panel opens, and when `sequence.json` in the data folder changes |
| Running sessions | native process scan (libproc + `KERN_PROCARGS2`): a `claude` process's `CLAUDE_CONFIG_DIR` gives its slot |
| Auto-switch | `haryx auto --json` as a child process, only when you turn it on; stopped on quit |
| Prompts | the app confirms first, then answers the CLI's y/N on stdin; tokens go via stdin (`add-token -`) |

### Performance
CPU is ~0.3% idle and ~4% while the menu bar mascot walks; that cost comes from AppKit re-snapshotting the status
item each frame. Turn it off under Preferences → "Animate menu bar mascot". In-app mascots animate only while
visible (4 fps idle, 8 fps running) from cached frames.

## License

MIT, see [LICENSE](LICENSE). The CLI is ported from claude-swap (MIT, Onur Cetinkol).
