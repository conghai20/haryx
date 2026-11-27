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
see each account's 5-hour and weekly usage, choose which account Claude Code is logged in as, sign
accounts in again, run Claude as a given account per folder or terminal, and carry your Claude Code
setup (skills, hooks, MCP servers, plugins, history) from one machine to another. It ships as:

- **`haryx`**, a command-line tool in Rust (`cli/`). Everything about AI coding accounts lives in the
  `ai` section (`haryx ai list`). It started as a drop-in port of
  [claude-swap](https://github.com/realiti4/claude-swap) (`cswap`) and keeps its commands, flags and
  `--json` output, plus an interactive dashboard (`haryx ai tui`).
- **HaryX for macOS**, a native menu bar app (`apps/macos/`) that drives `haryx` and bundles it.

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
- **Sync stays between your machines.** Signing in to HaryX (optional) syncs your settings and account setup
  between your own machines. Each machine signs in to Claude itself; a Claude browser login is never copied.
- **Anthropic decides.** Anthropic may warn, throttle, suspend or close accounts it believes break its
  terms. Read the [Consumer Terms](https://www.anthropic.com/legal/consumer-terms), the
  [Usage Policy](https://www.anthropic.com/legal/aup) and Claude Code's
  [legal and compliance](https://code.claude.com/docs/en/legal-and-compliance) page before you use HaryX.

## Install

**macOS app (includes the CLI).** Run:

```sh
curl -fsSL https://raw.githubusercontent.com/conghai20/haryx/main/scripts/install.sh | sh
```

It downloads the latest DMG, checks its SHA-256, installs `HaryX.app` in `/Applications` (replacing an
older copy) and opens it. After that HaryX updates itself (About & Updates). A download through `curl` is not marked as coming
from the internet, so macOS opens the app without asking for approval in Privacy & Security.
Options: `HARYX_VERSION=0.1.2` pins a version, `HARYX_INSTALL_DIR=~/Applications` changes the folder.

Or download `HaryX-<version>.dmg` from [GitHub Releases](https://github.com/conghai20/haryx/releases)
and drag HaryX to Applications. The DMG is not notarized yet, so on first open macOS blocks it: open
System Settings › Privacy & Security and click Open Anyway.

On first launch it offers to install the `haryx` command in `/usr/local/bin` (asks for your
password) or `~/.local/bin`; Preferences › Terminal command does the same later. The command is a
link into the app, so updating the app updates the command.

**Windows app (includes the CLI).** In PowerShell:

```powershell
irm https://raw.githubusercontent.com/conghai20/haryx/main/scripts/install.ps1 | iex
```

It downloads the latest `HaryX-<version>-<x64|arm64>-setup.exe`, checks its SHA-256 and installs HaryX
for your user (no admin rights), then starts it in the tray. After that HaryX updates itself (About &
updates). The installer is
not code-signed yet: a copy downloaded with a browser gets a SmartScreen warning (More info › Run
anyway), which the script avoids. `HARYX_VERSION=0.1.3` pins a version. Needs Windows 10 22H2 or 11.

**Updates.** Both apps check GitHub releases once a day and show "HaryX X is available" in the menu
bar panel or tray flyout. HaryX › About & Updates sets how much they do on their own: install
automatically (download in the background, install on quit), notify (the default) or only when you
check. Each download is checked against its SHA-256 before it replaces anything, and the bundled
`haryx` updates with the app. `haryx upgrade` updates a CLI you installed yourself: through its
package manager, or for a release tarball by downloading and checking the new one.

**CLI only.** Each release also has `haryx-v<version>-<target>.tar.gz` for macOS, Linux and Windows
(Apple silicon, Intel, ARM64, x86_64). Or build it:

```sh
cargo install --locked --git https://github.com/conghai20/haryx haryx
```

Homebrew (formula and cask), crates.io, PyPI (`pipx install haryx`) and npm (`npm i -g haryx`) are
coming; see [docs/release.md](docs/release.md).

> **Run one account switcher only.** Claude's refresh tokens rotate: if cswap and haryx (or two
> machines) both hold the same login, one renewal signs the other out ("re-login needed"). Move your
> accounts with `haryx ai import --from-cswap`, then stop using cswap.

## Quick start

```sh
haryx ai add                  # store the current Claude Code login
haryx ai login                # sign in another account (browser), the active login stays
haryx ai list                 # every account with its 5-hour and weekly usage
haryx ai switch 2             # make account 2 the Claude Code login (number, email or alias)
haryx ai switch --strategy best   # the account with the most quota left
haryx ai run 2 -- --resume    # run Claude as account 2 in this terminal only
haryx ai map 2                # always run as account 2 in this folder
haryx ai login 3              # fix "re-login needed" on account 3
haryx ai auto                 # opt-in: move to another of your accounts at your threshold
haryx ai tui                  # interactive dashboard (bare `haryx` in a terminal)
haryx ai help                 # every command
```

### Sync between your machines (optional)

```sh
haryx login                 # WeCoAp ID sign-in in your browser (--provider github|google|apple|email)
haryx sync unlock           # the first machine sets a sync passphrase; the others enter it
haryx sync                  # settings, app preferences, account settings, API-key and setup-token accounts
haryx sync profile push work   # a saved setup profile (secrets redacted, no history)
```

**What HaryX sends, and where.** Signed out: nothing new. Signed in, your HaryX account is your WeCoAp ID
(`sso.wecoap.com`), and the HaryX API (`haryx.wecoap.com`) keeps your data; both run on servers in
Vietnam. Sync needs your consent (`haryx account consent grant sync`). Everything synced is end-to-end
encrypted with a key only your passphrase opens, under row names the server cannot read: settings, app
preferences, account settings, API-key and setup-token accounts, setup profiles, and a monthly usage
summary per machine (tokens by day, account and model, and 5h/7d readings; never a project path, a
session id or conversation text). Showing your machines on HaryX web (name, OS, HaryX version, last seen)
is a second, separate consent. Withdrawing a consent erases that data on the server; `haryx account
export` downloads everything kept about you. Claude browser logins and conversations never leave your
machine. Accounts on another machine are listed so you can sign them in here.

**HaryX web** (`https://haryx.wecoap.com/platform/`) shows your machines and consents, and, once you
enter your sync passphrase in the browser, your accounts, usage and profiles. The server never sees the
passphrase or the key.

### Move your Claude Code setup to another machine

```sh
haryx ai profile show                              # what ~/.claude holds, with problems flagged
haryx ai profile export work.haryx-profile         # secrets redacted; --secrets encrypt to keep them
haryx ai profile import work work.haryx-profile --dry-run   # on the other machine: what would change
haryx ai profile import work work.haryx-profile --apply     # apply it (backup first)
haryx ai profile rollback                          # undo the last apply
```

A profile carries settings, hooks, skills, agents, commands, rules, plugins, MCP servers, memory
files, other `~/.claude` files and history (`--no-history` leaves it out). Logins, signed files, the
whole `~/.claude.json` and caches are opt-in. Everything that can run code is listed for
confirmation before it is applied. Details: [docs/ai-profile-plan.md](docs/ai-profile-plan.md).

## Repository

| Path | What |
|---|---|
| [`cli/`](cli) | `haryx` CLI and dashboard (Rust workspace: `haryx-core`, `haryx`) |
| [`apps/macos/`](apps/macos) | HaryX macOS app (SwiftUI, macOS 26+) |
| [`apps/windows/`](apps/windows) | HaryX Windows tray app (Tauri 2 + Svelte 5) |
| [`packaging/`](packaging), `scripts/release/` | PyPI and npm wrappers, release tarballs, Homebrew formula |
| [`packages/contracts/`](packages/contracts) | JSON contracts shared by the app and the CLI |
| `assets/brand/`, `scripts/brand/` | logo sources, exports and generators |
| [`upstream/`](upstream) | local reference copy of claude-swap (MIT) for the Rust port |
| [`docs/`](docs) | [architecture](docs/architecture.md), [roadmap](docs/roadmap.md), brand notes |

## CLI

```sh
cd cli
cargo test --workspace      # tests
cargo run -p haryx -- ai list  # run from source
cargo install --path crates/haryx   # → ~/.cargo/bin/haryx
```

See [cli/README.md](cli/README.md) for the commands. haryx keeps its own data folder
(`~/Library/Application Support/HaryX`, `$XDG_DATA_HOME/haryx` on Linux, `%LOCALAPPDATA%\haryx` on
Windows, or `HARYX_HOME`);
`haryx ai import --from-cswap` copies an existing cswap setup and leaves cswap untouched.

## Windows app

A tray icon with a flyout (the active account's usage, the next accounts with Use, Switch, a
terminal menu) and a dashboard (accounts, running sessions, auto-switch, wake, setup profiles,
tools, settings), over the same `haryx` CLI. Windows 10 22H2 or 11, x64 or ARM64.

```sh
cd apps/windows
npm ci
npm test && npm run check            # UI tests and type check
(cd src-tauri && cargo test)         # backend tests
npx tauri dev                        # run from source (also runs on macOS for UI work)
npx tauri build                      # NSIS installer (on Windows)
```

Plan and design notes: [docs/windows-app-plan.md](docs/windows-app-plan.md).

## macOS app

Requirements: macOS 26+. Building needs the Xcode 26+ toolchain. The app uses an installed `haryx`
first, then the one bundled in the DMG build, and falls back to `cswap` ≥ 1.1.

```sh
cd apps/macos
swift test                  # core tests
scripts/build.sh            # → apps/macos/build/HaryX.app
scripts/build.sh --install  # → /Applications/HaryX.app, launched
scripts/package-dmg.sh      # → apps/macos/build/HaryX-<version>.dmg with haryx bundled
```

### What it does
- **Menu bar**: active account's mascot plus its 5-hour %. The mascot walks while a Claude session is using that account.
  Click it to open a glass panel with an **AI** section: all accounts, usage bars, Switch (best / next / next-available),
  Auto-switch, Refresh, **+** to sign in a new account, and terminal quick actions.
- **Sign in**: **Sign In Again** on accounts that need a re-login and **Sign In New Account**. Sign-in runs in the
  background in its own window with the sign-in link (Open / Copy), a field for the code, and the outcome.
- **Dashboard**: account cards plus an inspector with every CLI control: switch / force switch, alias,
  enable/disable, threshold, priority, move, swap, remove, run Claude as the account, live sessions, mascot species
  and colour. Also panes for auto-switch (live event log), folder mappings, every `config` key,
  export/import/import-usage, unclaimed credentials, upgrade and purge.
- **Setup Profiles**: browse the live `~/.claude` and saved profiles with status badges (broken hook, plain secret,
  missing secret), save snapshots, export with redacted or encrypted secrets, import a file, review the dry-run plan
  and the code it installs, apply, delete, and undo the last apply.
- **Terminal command**: installs the bundled `haryx` on your PATH (first launch, or Preferences).
- **Mascot moods**: idle (breathe + blink) · running (walk) · tired (≥90% quota, sweat) · sleeping (disabled) · error (re-login needed).
- **Terminals**: Terminal, iTerm2, Warp, Ghostty, kitty, WezTerm, Alacritty (installed ones are listed), plus shell choice.
  "Run Claude as account N" uses `haryx ai run N` (session mode, that terminal only).
- **Notifications**: account switched, usage 80/95% or projected run-out, re-login / expiring login / expired plan,
  quota reset, Claude session finished (≥1 min), auto-switch quarantine / all exhausted / errors. Each can be toggled.

### How it works
| | |
|---|---|
| Data | `haryx ai list --json`, polled (default 60 s), when the panel opens, and when `sequence.json` in the data folder changes |
| Running sessions | native process scan (libproc + `KERN_PROCARGS2`): a `claude` process's `CLAUDE_CONFIG_DIR` gives its slot |
| Auto-switch | `haryx ai auto --json` as a child process, only when you turn it on; stopped on quit |
| Prompts | the app confirms first, then answers the CLI's y/N on stdin; tokens go via stdin (`add-token -`), profile passphrases via the environment |
| Profiles | `haryx ai profile … --json` (contracts in `packages/contracts/haryx/profile-*.json`) |

### Performance
CPU is ~0.3% idle and ~4% while the menu bar mascot walks; that cost comes from AppKit re-snapshotting the status
item each frame. Turn it off under Preferences → "Animate menu bar mascot". In-app mascots animate only while
visible (4 fps idle, 8 fps running) from cached frames.

## License

MIT, see [LICENSE](LICENSE). The CLI is ported from claude-swap (MIT, Onur Cetinkol).
