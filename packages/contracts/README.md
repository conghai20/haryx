# contracts

Language-neutral data contracts shared by every HaryX part (macOS app, `haryx` CLI, and later server, web, iOS,
Android). Add or change a shape here first, then update the code that reads or writes it.

| Path | What | Checked by |
|---|---|---|
| `haryx/upgrade-check.json` | `haryx upgrade --check --json`: the latest release, its notes and this machine's app and CLI downloads | `cli/crates/haryx/src/commands/upgrade.rs`, both apps' tests |
| `haryx/whoami.json` | `haryx whoami --json` and `haryx logout --json`: whether this machine is signed in to a HaryX account. Signed out is `{"signedIn": false, "user": null}`. `provider` is `github` or `google`. Never carries a token. | `cli/crates/haryx/tests/account.rs` |
| `haryx/login-events.json` | `haryx login --json`: one JSON object per line, in this order. `browser` (the sign-in URL; `opened` is false with `--no-browser` or when no browser could start), then `signed_in` or `error`. Error `code`: `timeout`, `denied`, `refused`, `network`, `storage`. | `cli/crates/haryx/tests/account.rs` |
| `haryx/sync-result.json` | `haryx sync --json`. On failure `{"ok": false, "code": …, "message": …}` with exit 1; `code` is `not_signed_in`, `locked`, `wrong_passphrase`, `key_changed`, `busy`, `invalid`, `local`, `server`, or a sign-in code (`network`, `signed_out`, …). `changed` lists kinds that changed this machine: `settings`, `app-prefs`, `account`, `credential`. `lost` is null when the losing side had the value unset, and always null for tokens. | `cli/crates/haryx/tests/sync.rs` |
| `haryx/sync-status.json` | `haryx sync status --json`: local only, no network. `lastSyncAt` is null before the first sync. | `cli/crates/haryx/tests/sync.rs` |
| `haryx/app-prefs.json` | `haryx sync prefs get` / `set` (stdin): the apps' synced preferences, one flat object. Shared keys have no prefix; `macos.*` and `windows.*` are read only by that app. The CLI stores and merges it per key without interpreting it. Never put a path or a secret here. Notifications use the apps' category names (`notifications.switched`, `usage`, `login`, `reset`, `session`, `autoSwitch`); `mascotStyles` maps `email|organizationUuid` to `{species, hue}` (hue optional), species one of the eight shared sprites. | the apps |
| `haryx/sync-profiles.json` | `haryx sync profile list --json`: setup profiles kept on the server (3 at most). | `cli/crates/haryx/tests/sync.rs` |
| `cswap/list.json` | `cswap list --json` / `haryx list --json` payload (schemaVersion 1, from cswap 1.1.0, anonymised) | `apps/macos/Tests/HaryXCoreTests` |

The Ruby CLI must produce output that matches these fixtures (drop-in compatibility with cswap).
