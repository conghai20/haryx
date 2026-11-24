# haryx JSON fixtures

Payloads captured from the Rust `haryx` CLI (`cli/`), checked by the macOS app tests so the app
keeps decoding haryx output as well as cswap's (`../cswap`).

- `list-unavailable.json` — `haryx list --json` in a sandbox with two accounts and the usage
  endpoint unreachable (`usage: null`, `usageError`, `usageRetryAt`).
- `profile-list.json` — `haryx ai profile list --json`: saved profiles (the live `default` is implicit).
- `profile-show.json` — `haryx ai profile show --json`: inventory items with `category`, `id`, `kind`
  (`file`, `folder`, `key`, `mcp`, `plugin`, `accounts`), `size`, `files`, `status` (`ok`, `broken`,
  `secret`, `needs-secret`) and `detail`.
- `profile-plan.json` — `haryx ai profile apply NAME --dry-run --json`: one entry per item with `action`
  (`add`, `update`, `merge`, `remove`, `same`, `skip`), plus `runs_code`, `missing_requires`, `warnings`.
- `profile-diff.json` — `haryx ai profile diff NAME --json`: `state` per item (`only-left`, `only-right`,
  `differs`, `same`).
- `wake-status.json` — `haryx ai wake status --json` (synthetic accounts): settings (`accounts` opted in by
  number, `quietHours`, `weeklyCapPct`), `agentInstalled`, one row per account (`on`, `lastWakeAt`,
  `lastCheckAt`, `nextDue`, `lastResult`: `woke`, `not-due`, `skip:<reason>`, `failed:<kind>`) and the last
  `events`.
- `sessions.json` — `haryx ai sessions --json` (synthetic): every live Claude Code session with `pid`,
  `account` (slot number, `null` when a default-profile session has no active account), `email`, `profile`
  (`default` for `~/.claude`, `session` for a slot's own profile), `sessionId`, `cwd`, `startedAt` (epoch ms),
  `kind`, `entrypoint`, `status` (`busy`, `idle`, `waiting` or `null`), plus `unreadable` (records that
  could not be read). Read from Claude Code's `sessions/{pid}.json` records, so it works on every OS; the
  Windows app uses it in place of the macOS app's native process scan.

