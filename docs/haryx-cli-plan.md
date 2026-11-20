# `haryx` CLI: Rust rebuild of claude-swap

Status: **in progress**. Working: `config`, `list`, `status`, `switch` (all strategies, `--force`), `add`, `add-token`, `remove`, `enable`/`disable`, `alias`, `threshold`, `priority`, `swap`, `move`, `map`/`unmap`, `unclaimed`, `purge`, `export`, `import` (incl. `--from-cswap`), `import-usage`, `run`, `auto` (loop, `--once`, `--json`, `--dry-run`, all strategies), `upgrade` (Homebrew, cargo, pipx, uv, npm), `tui`/`watch` (ratatui: dashboard, switch, watch, auto view). Every upstream command is ported; the macOS app drives haryx; packaging for every channel is in place (see `docs/release.md`). Next: first tagged release, real-account checks.

Source of truth for behaviour: `upstream/claude-swap/` (1.2.0b1, MIT, pinned in `upstream/README.md`). Its `docs/architecture.md` is the module map this port follows.

## Decisions (confirmed with the owner, 2026-11-20)

| Topic | Decision |
|---|---|
| Language | Rust (stable, 1.100+). Replaces the earlier Ruby plan. |
| Behaviour target | claude-swap **1.2.0b1**: every command, flag, prompt and `--json` shape. |
| Command surface | Drop-in: `haryx <cmd>` accepts what `cswap <cmd>` accepts. Program name in messages is `haryx`. |
| Data store | **Own store**, not shared with cswap. macOS `~/Library/Application Support/HaryX`, Linux `$XDG_DATA_HOME/haryx` (default `~/.local/share/haryx`). `HARYX_HOME` overrides (tests, portable use). Keychain items for slot backups use the `haryx` service prefix. |
| cswap users | `haryx import --from-cswap` copies (never moves) a cswap store, including Keychain backups. On first run with an empty store, a cswap store is detected and the import is offered (interactive y/N at a terminal; a one-line stderr hint otherwise; never on `--json` stdout). |
| Live login | Unchanged and shared by necessity: haryx reads and writes the same live credential and `.claude.json` that Claude Code uses, under Claude Code's own locks. |
| Platforms | macOS + Linux in v1. Windows stays on the roadmap; platform code is `cfg`-gated so it can be added. |
| Scope of v1 | Everything except the Python menu bar (the HaryX macOS app replaces it): core commands, `auto`, `run`/sessions, `watch`, TUI. Upstream's launch-agent service exists only to run that menu bar, so it is dropped with it. |
| Layout | Self-contained Cargo workspace in `cli/` (the future server is Go). |

## Layout

```
cli/
├── Cargo.toml                 # workspace
├── crates/
│   ├── haryx-core/            # library: everything except argument parsing and terminal rendering
│   │   └── src/
│   │       ├── paths.rs       # Claude Code path rules + haryx store root          (paths.py)
│   │       ├── fsutil.rs      # atomic writes, permissions                         (fsutil.py)
│   │       ├── locking.rs     # haryx FileLock                                     (locking.py)
│   │       ├── claude_locks.rs# Claude Code's lock protocol                        (claude_locks.py)
│   │       ├── models.rs      # slots, aliases, thresholds, priorities             (models.py)
│   │       ├── settings.rs    # settings.json + `config`                           (settings.py)
│   │       ├── sequence.rs    # sequence.json store                                (switcher.py, storage half)
│   │       ├── credentials/   # CredentialStore: file + macOS `security`           (credentials.py, macos_keychain.py)
│   │       ├── oauth.rs       # token / usage / profile endpoints                  (oauth.py)
│   │       ├── usage/         # UsageStore, poll policy, pace, subscription        (usage_store.py, poll_policy.py, pace.py, subscription.py)
│   │       ├── switcher/      # add, remove, switch, refresh gate, list/status     (switcher.py)
│   │       ├── json_output.rs # --json shapes                                      (json_output.py)
│   │       ├── transfer.rs    # export / import / import --from-cswap              (transfer.py)
│   │       ├── session.rs     # `run`, profiles, mappings                          (session.py, mappings.py)
│   │       ├── process.rs     # running Claude Code detection                      (process_detection.py, spawn.py)
│   │       └── autoswitch/    # engine + events                                    (autoswitch.py, lines.py)
│   └── haryx/                 # binary: clap CLI, printer, TUI (ratatui), watch
└── tests/                     # integration tests against a sandboxed HOME
```

Main crates: `clap` (derive), `serde`/`serde_json`, `thiserror`/`anyhow`, `ureq` with rustls and the platform
verifier (the counterpart of `truststore`), `jiff` (dates), `fs4` (file locks), `libc` (process scan),
`ratatui`/`crossterm` (TUI). The macOS Keychain is driven through `/usr/bin/security`, as upstream does, so item
ACLs stay compatible with Claude Code.

## Rules carried over from upstream

Lock order (haryx, then Claude credential lock, then Claude config lock); no network under a lock except the
documented token refresh; a used refresh token's successor is always persisted; a live credential enters a slot
only when it is known to belong there; collection and the auto tick never fail outward; paths only from `paths.rs`.

## Phases

Each phase ends green: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`.

| # | Phase | Delivers |
|---|---|---|
| 0 | Scaffold | Workspace, error type, logging (`--debug`), full clap command tree with cswap's subcommand aliases, `--version` |
| 1 | Foundation | paths, fsutil, locking, Claude locks, models + validators, settings + `config` |
| 2 | Store and credentials | sequence.json, CredentialStore (file, Keychain), slot backup read/write |
| 3 | Usage | oauth endpoints, UsageStore with leases and backoff, poll policy, pace, plan labels |
| 4 | Read commands | `list`, `status` (text + `--json` matching `packages/contracts/`) |
| 5 | Mutations | add, add-token, remove, enable/disable, alias, threshold, priority, move, swap, map/unmap, unclaimed, purge, switch (all strategies), refresh gate |
| 6 | Transfer | export, import, import-usage, `import --from-cswap`, first-run offer |
| 7 | Sessions | `run`, process detection, spawn |
| 8 | Auto-switch | engine, `auto` (`--once`, `--json` events, strategies, dry run) |
| 9 | Watch, TUI, upgrade | `watch`; ratatui dashboard; `upgrade` against HaryX releases |
| 10 | Integration | macOS app calls `haryx` (falls back to `cswap`) and watches the haryx store; contract fixtures refreshed to 1.2.0b1; packaging (Homebrew, crates.io, npm/pip binary wrappers via cargo-dist) |

## Testing

- Unit tests per module, ported from the matching `upstream/claude-swap/tests/test_<module>.py` cases.
- Integration tests run the binary with `HOME`, `HARYX_HOME`, `CLAUDE_CONFIG_DIR` in a temp dir, a fake
  `security` on `PATH`, and a local HTTP stub for the OAuth/usage endpoints (`HARYX_API_BASE`, test-only).
- Contract tests: `list --json`, `status --json`, `switch --json` and `auto --json` events validate against
  `packages/contracts/`.
- Tests can never touch the real store: the binary refuses to write a real store when `HARYX_TEST_GUARD` is set.

## Open checks before release

- **User-Agent:** haryx sends `haryx/<version>` (upstream: `claude-swap/1.0`). Both are non-first-party, so the
  usage endpoint's request budget should behave the same, but this is unmeasured: once `haryx list` works on real
  data, watch one real account for unexpected 429s over a few hours.
- **Terms (reviewed 2026-11-21, not legal advice):** sources are Anthropic's Claude Code
  [Legal and compliance](https://code.claude.com/docs/en/legal-and-compliance) page, the
  [log-in help article](https://support.claude.com/en/articles/13189465-log-in-to-your-claude-account), the
  [Usage Policy](https://www.anthropic.com/legal/aup) and the Consumer Terms.
  - *Fits:* subscription OAuth is for the subscriber's own, ordinary use of Claude Code. haryx never sends model
    requests, never offers a "log in with Claude" of its own and never resells or intermediates usage; the
    unmodified Claude Code binary does the work with each account's own login, and the binary's authentication is
    not removed or disabled.
  - *Risk 1, limit rotation:* the help article says tools that "attempt to route third-party traffic against
    subscription limits" are prohibited, and the docs say advertised limits "assume ordinary, individual usage".
    The Usage Policy forbids creating multiple accounts "to avoid detection, circumvent product guardrails", aimed
    at malicious use. Auto-switching to keep working past one plan's limit is the closest thing to this: the
    biggest exposure, and Anthropic may read it as circumvention.
  - *Risk 2, identity:* tools that "misrepresent their identity to Anthropic's servers" are prohibited. haryx
    already sends its own User-Agent, but token refresh reuses Claude Code's OAuth client id, and the usage
    endpoint is undocumented.
  - *Risk 3, sharing:* the account holder is responsible for all activity on the account; one person's own
    accounts only, never a team sharing logins.
  - *Before going public:* state these limits in the README (personal accounts only, no sharing, the user's own
    responsibility under Anthropic's terms); keep `auto` opt-in and describe it as balancing the user's own plans,
    not as getting around limits; ideally ask Anthropic whether multi-account switching and the usage endpoint
    are acceptable. Enforcement is a warning, throttling, suspension or termination of the user's accounts.
