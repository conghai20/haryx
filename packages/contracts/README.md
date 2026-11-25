# contracts

Language-neutral data contracts shared by every HaryX part (macOS app, `haryx` CLI, and later server, web, iOS,
Android). Add or change a shape here first, then update the code that reads or writes it.

| Path | What | Checked by |
|---|---|---|
| `haryx/upgrade-check.json` | `haryx upgrade --check --json`: the latest release, its notes and this machine's app and CLI downloads | `cli/crates/haryx/src/commands/upgrade.rs`, both apps' tests |
| `haryx/whoami.json` | `haryx whoami --json` and `haryx logout --json`: whether this machine is signed in to a HaryX account. Signed out is `{"signedIn": false, "user": null}`. `provider` is `github` or `google`. Never carries a token. | `cli/crates/haryx/tests/account.rs` |
| `haryx/login-events.json` | `haryx login --json`: one JSON object per line, in this order. `browser` (the sign-in URL; `opened` is false with `--no-browser` or when no browser could start), then `signed_in` or `error`. Error `code`: `timeout`, `denied`, `refused`, `network`, `storage`. | `cli/crates/haryx/tests/account.rs` |
| `cswap/list.json` | `cswap list --json` / `haryx list --json` payload (schemaVersion 1, from cswap 1.1.0, anonymised) | `apps/macos/Tests/HaryXCoreTests` |

The Ruby CLI must produce output that matches these fixtures (drop-in compatibility with cswap).
