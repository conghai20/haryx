# Changelog

All notable changes to HaryX (the `haryx` CLI and the macOS app) are recorded here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- Account, usage and session commands now live in the `ai` section: `haryx ai list`,
  `haryx ai switch 2`, `haryx ai tui`, `haryx ai watch`, and so on. `haryx --help` lists the
  sections; `haryx ai help` lists the commands. Hints and help text use the `ai` form.
  The cswap spellings without `ai` (`haryx list`, `haryx --list`) keep working, so scripts written
  for `cswap` are unaffected. Bare `haryx` in a terminal still opens the dashboard.
- macOS app: the menu bar panel and the dashboard sidebar group everything about AI coding
  accounts under an **AI** section; the sidebar's app preferences sit under **HaryX**.

### Added

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

[Unreleased]: https://github.com/conghai20/haryx/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/conghai20/haryx/releases/tag/v0.1.0
