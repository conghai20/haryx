# Roadmap

## Rust `haryx` CLI
Rebuild every claude-swap command in Rust (`cli/`, plan in `haryx-cli-plan.md`), drop-in compatible with `cswap`: same commands, flags,
prompts and `--json` shapes, verified against `packages/contracts/`. Port from `upstream/claude-swap/`
(MIT, pinned in `upstream/README.md`). Then switch the macOS app to call `haryx`, falling back to `cswap`.

## Setup profiles
`haryx ai profile`: list, save, export, import and apply a Claude Code setup (skills, hooks, MCPs, plugins)
across machines through one file. Plan: [ai-profile-plan.md](ai-profile-plan.md).

## Claude Desktop follows Use
Pressing Use in the macOS app also moves Claude Desktop to that account, through a per-account Desktop data folder.
Blocked on one sign-in test. Plan: [desktop-follow-plan.md](desktop-follow-plan.md).

## Continue a session on another account
When an account runs out, end its stuck Claude Code session and resume the same conversation on
another account in one click; offered after auto-switch. Plan: [session-handoff-plan.md](session-handoff-plan.md).

## Usage history
Tokens and API-price cost by account, project, session and model, quota % over time, and a weekly
summary, from Claude Code's own transcripts, kept locally. Plan: [usage-history-plan.md](usage-history-plan.md).

## Distribution
Homebrew (formula + cask), crates.io, Developer ID signed and notarized DMG on GitHub releases, and pip/npm wrapper
packages for the CLI on macOS, Linux and Windows.

## Future parts
- **Server** (Go): sync and team view; receives metadata only, never credentials.
- **Web** dashboard.
- **iOS and Android**, native.
- **DevSecOps / SRE / Cloud** areas: undecided; after the AI-accounts core ships.

## Windows support for the `haryx` CLI
The Rust CLI targets macOS, Linux and Windows, but these parts are macOS/Linux-specific today and need a Windows design:
- **Credential storage:** upstream uses the macOS Keychain / Linux files; pick a Windows store (Credential Manager or a protected file).
- **Running-session detection:** `CLAUDE_CONFIG_DIR` per process is read via libproc / `/proc` on macOS and Linux; Windows needs a process-environment equivalent or a different signal.
- **Terminal launch (`run`, quick actions):** map to Windows Terminal, PowerShell and cmd.
- **Paths:** `~/.claude-swap-backup` and `~/.claude` locations under `%USERPROFILE%`.
- **Packaging:** Windows builds of the prebuilt binaries for the pip/npm wrappers.

All five are built (2026-11-24), together with the Windows tray app on top; see
[windows-app-plan.md](windows-app-plan.md) for what changed from the plan and what is still open.
