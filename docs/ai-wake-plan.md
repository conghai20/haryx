# `haryx ai wake`: auto-restart the 5-hour window

Status: CLI implemented (2026-11-24); macOS app surfaces ship with the brand redesign.

Built as planned, with three differences:
- Settings and state live in `wake.json` in the haryx store, not in `settings.json` keys.
- The wake's response headers set the next due time but are not written to the usage store.
- The LaunchAgent label is `dev.haryx.wake`; it carries `HARYX_HOME`, `CLAUDE_CONFIG_DIR` and
  `XDG_DATA_HOME` when set.

## Goal

When an opted-in account's 5-hour window resets, haryx starts the next window right away with the
smallest possible request. The window clock then runs while the user is away, so a fresh window is
closer to its own reset when work resumes.

## Decisions (from the owner)

| Topic | Decision |
|---|---|
| Meaning | Restart the 5-hour window. No Claude Code session is resumed. |
| Accounts | Opt-in per account, off by default. |
| Timing | Fire at `resets_at`. Skip inside quiet hours and when weekly usage is at or above a cap. |
| Runner | CLI tick run by a user LaunchAgent. The app only shows status and toggles settings. |
| Mac asleep | Fire late, on the first tick after wake. No root, no `pmset`. |
| Weekly cap | Skip at 7-day usage >= 90% (configurable). Skip when the window already started on its own. |
| Notify | Event log plus app status. A macOS notification only on failure. |

## Cost

One wake = the existing header-probe request (`oauth::USAGE_PROBE_*`): Haiku, `max_tokens: 1`, the
required Claude Code system prompt and a one-character user message. About 20 input tokens and
1 output token. Every check that does not end in a wake costs zero tokens: it reads the stored
usage or the free usage endpoint.

## Tick algorithm

`haryx ai wake run --once`, for each opted-in, enabled account, under one file lock:

1. Read the stored usage entry. If the 5-hour `resets_at` is in the future, the account is not due:
   record `next_due = resets_at` and stop. No network.
2. Due (reset passed, or no window reported). Refresh the reading: usage endpoint for OAuth logins
   (free); setup-token accounts skip this step because their read is the probe itself.
3. Fresh reading shows an active window (new `resets_at` in the future): skip `already-active`.
4. Now inside quiet hours: skip `quiet-hours`. The next tick after quiet hours end fires.
5. 7-day usage >= cap: skip `weekly-cap`.
6. Login expired or rejected: skip `login`, notify once per window.
7. Send the wake request. Store the response headers as a new usage reading (the wake doubles as
   a usage refresh). Record `last_wake_for = <consumed resets_at>` so a window is never woken twice.

The decision (steps 1, 3 to 6) is a pure function `decide(entry, now, config) -> Decision` and
holds most of the tests. Several machines waking the same account are safe: the second one sees
the active window in step 3.

The active account's credentials come from the same live source the usage read already uses
(`switcher/usage_active.rs`), so a token Claude Code rotated is not overwritten.

## Command surface

```
haryx ai wake on <account>... | all   opt in
haryx ai wake off <account>... | all  opt out
haryx ai wake status [--json]        per account: on/off, next due, last wake, last skip reason
haryx ai wake config [--quiet 01:00-06:00 | --no-quiet] [--weekly-cap 90]
haryx ai wake run [--once] [--dry-run] [--json]
haryx ai wake install | uninstall    the LaunchAgent (macOS); prints what it writes
```

`all` is a snapshot: `on all` opts in every account that exists now, and accounts added later stay
off (opt-in by default holds). `off all` clears the list. `all` is a reserved word: `alias` refuses
it, and `all` mixed with account names is an error. The output names each account it changed.

Settings keys: `wake.accounts`, `wake.quietHours`, `wake.weeklyCapPct`. Events append to
`wake-events.jsonl` next to the usage store, capped like the auto-switch event log.

## LaunchAgent

`~/Library/LaunchAgents/<bundle-id>.wake.plist`, `StartInterval` 60 s, `RunAtLoad`, running
`haryx ai wake run --once`. launchd coalesces intervals missed during sleep into one run after
wake, which gives "fire late" for free. Installed with `launchctl bootstrap gui/<uid>`.
`service.rs` is a stub today; this feature implements it. Linux (systemd user timer) comes later.

## macOS app

- Account row: a wake toggle and a "wakes 14:05" / "skipped: weekly cap" line.
- Settings: quiet hours, weekly cap, LaunchAgent install state with Install/Remove.
- Reads `haryx ai wake status --json`; writes through the CLI only.

## Phases

1. Core `wake` module: config, `decide`, state (`last_wake_for`, last skip), event log. TDD.
2. Sender: reuse the probe request and header parsing; store the reading.
3. CLI `haryx ai wake` parser, views, `--json` contract fixtures in `packages/contracts`.
4. LaunchAgent install/uninstall in `service.rs`.
5. macOS app surfaces.
6. README, CHANGELOG.

## Open questions (defaults in brackets)

- Tick interval: 60 s (decided). A wake fires at most about a minute after reset. A tick with no
  due account only reads local files: a few milliseconds of CPU, no network, no tokens. A
  `GRACE_S` of about 5 s after `resets_at` keeps the wake from landing before the server rolls the
  window. If minute precision is ever not enough, `run` can become a resident process that sleeps
  until `min(next_due, 60 s)` and re-checks the wall clock after each sleep (monotonic timers
  pause while the Mac sleeps).
- Default quiet hours [none; the user sets them].
- Linux timer in this feature or later [later].
