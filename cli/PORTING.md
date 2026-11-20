# Porting rules (claude-swap → haryx)

Read `docs/haryx-cli-plan.md` (decisions, layout, phases) and `upstream/claude-swap/docs/architecture.md`
(module map, cross-module flows, invariants) first.

## Faithfulness
- Port behaviour, not just shape: every branch, edge case, constant, message and file format of the upstream
  module. The upstream docstrings and comments explain *why*; keep the important ones as Rust doc comments.
- Same on-disk formats (JSON keys, file names under the store root) and the same `--json` output, so the
  macOS app and the contracts in `packages/contracts/` keep working.
- Rename only what the plan renames: program name `cswap` → `haryx` (use `haryx_core::PROG`), store root
  (see `paths.rs`), Keychain service prefix for slot backups (`haryx`), env overrides `HARYX_*`.
- macOS + Linux only. Windows branches become `#[cfg(windows)]` stubs returning a clear "not supported yet"
  error, or are left out with a `// TODO(windows):` note naming the upstream function.
- Skip: the Python menu bar (`menubar.py`, `app_bundle.py`), `migrations.py` (haryx starts with the current
  layout; `import --from-cswap` reads the current cswap layout only).

## Code
- Errors: `haryx_core::Error` / `Result` (`error.rs`), one variant per upstream exception.
- No `unwrap()`/`expect()` outside tests except on proven invariants (comment why).
- Sync code only (no async runtime). HTTP via `ureq`. Dates via `jiff`. File locks via `fs4`.
- Immutable by default; small functions; files under ~800 lines (split a large upstream module into a
  directory module, e.g. `switcher/`).
- Tests: port the meaningful cases from `upstream/claude-swap/tests/test_<module>.py` as `#[cfg(test)]` unit
  tests or `tests/*.rs`. Tests never touch the real home: use `tempfile` and pass paths/env explicitly.
- Each phase ends with `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all green.
