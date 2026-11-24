# Usage history and "where did my quota go": plan

Status: built (2026-11-24): CLI, macOS Usage section, Windows Usage pane. Tested on this Mac
against ~190 MB of real transcripts (indexed in about 2 s).

Built as planned, with these differences:
- The settings section is `history` (`history.enabled`, `history.import`, `history.keepDays`),
  since `usage` already holds the usage-endpoint probes.
- The price table is `cli/crates/haryx-core/data/anthropic-pricing.json` (inside the crate so it
  embeds at build and survives packaging), not `packages/pricing/`. The staleness reminder is a
  unit test that fails once the table is 120 days old, so CI catches it.
- The apps read cost from the CLI's JSON; they carry no price table.
- Quota readings are appended by the usage store whenever it records a new measurement, so
  every reading the app or `auto` takes is kept, not only the ones seen during an index.
- `ai usage` runs the index first on every call, so the apps need no separate index step.
- Session rows in the apps open a terminal in project rows' folders; "continue this session" from
  the Usage tables is left out (the Sessions views have it, for live sessions).

Decisions taken with the owner on 2026-11-24:
- Show tokens **and** what they would cost at API prices (a price table kept in the repo).
- How much past history to read, and how long to keep it, is **the user's choice**: asked once
  when the feature first opens, and a setting after that.

## What already exists

- Every Claude Code transcript line for a model reply carries `message.usage` (input, output,
  cache-read and cache-write tokens, 5-minute and 1-hour cache writes), `message.model`,
  `timestamp`, `cwd`, `sessionId`, `requestId`. Transcripts live under each profile's `projects/`.
- They do **not** record the account. HaryX knows it from: `haryx.log` ("Switched from account X to
  Y" with a time) for the default profile, and the profile folder for `haryx run` sessions.
- The usage store (`cache/usage.json`) keeps only the latest 5h/7d reading per account.
- The apps poll `ai sessions` every few seconds, so they see which session runs as which account.

## Data (all local, no conversation content)

Under `<store>/usage/`:
- `index.json`: per transcript file, the byte offset read so far (and size/mtime), so each run
  reads only new lines. A shrunk or replaced file is read again from the start.
- `days/YYYY-MM-DD.jsonl`: rows aggregated per (hour, account, project, session, model): replies,
  input, output, cache read, cache write 5m, cache write 1h. Project = the session's `cwd`.
- `sessions.jsonl`: session id → account, profile, first/last seen. Written by every
  `ai sessions` scan (the apps run it constantly) and by `haryx run`; exact attribution.
- `quota.jsonl`: each new 5h/7d reading per account (time, %, resets_at), appended by the usage
  store when it records a measurement; unchanged readings are skipped.

Attribution order for a reply: `sessions.jsonl` (exact) → the profile folder it was found in →
the account active at that time from `haryx.log` (approximate; marked so) → `unknown`.

Duplicate lines: Claude Code writes one line per content block with the same usage; rows are
counted once per `requestId` (falling back to `message.id`).

## Settings (`haryx config`, shown in both apps)

- `usage.history.import`: `all` (read every transcript on disk on the first run) or `none` (from now
  on). Asked once in the apps' Usage pane before the first index ("Read your past Claude Code
  history? It stays on this machine."); the CLI's first `ai usage` asks the same in a terminal and
  defaults to `none` when not interactive.
- `usage.history.keep_days`: `30`, `90` (default), `365`, or `0` (forever). Older day files are
  deleted at the end of each index run. Changing it to a longer value does not bring back deleted
  days; re-importing (`ai usage index --rebuild`) reads the transcripts again.
- `usage.history` off entirely (`usage.history.enabled = false`): no index, no files.

## Prices

`packages/pricing/anthropic.json`: per model id prefix, USD per million tokens for input, output,
cache read, 5-minute cache write, 1-hour cache write, with an `asOf` date and the source URL. Filled
from Anthropic's published pricing when built (not from memory), and updated with each release
when it changes; a CI check fails when `asOf` is older than 120 days, as a reminder. Embedded in the
CLI at build time. A model not in the table shows tokens and "—" for cost. Every cost is labelled
"at API prices, as of <date>": plans are flat-priced, so it shows what the usage is worth, not
what was billed.

## Phase 1: CLI

- `haryx ai usage index [--rebuild]`: read new transcript lines (all profiles), update the day
  files, apply retention. Holds a lock; safe to run from both apps and the CLI at once. Fast path:
  a line without `"usage"` is skipped before JSON parsing.
- `haryx ai usage report [--since 7d|30d|DATE] [--by account|project|session|model|day]
  [--account N] [--project PATH] [--json]`: runs `index` first, then prints a table (tokens, cost,
  share). Default: last 7 days by account.
- `haryx ai usage quota [--account N] [--since 7d] [--json]`: the 5h/7d readings over time.
- `haryx ai usage summary [--week] [--json]`: the weekly digest ("Account 2 ran out 3 times, 61% of
  its tokens went to ~/work/api, about $140 at API prices").
- Contracts: `packages/contracts/haryx/usage-report.json`, `usage-quota.json`, `usage-summary.json`.

## Phase 2: macOS app

A **Usage** section in the AI group (Swift Charts):
- Quota over time: 5h and 7d % per account (lines), limits marked, exhaustions dotted.
- Tokens per day, stacked by account; switch between tokens and cost.
- Top projects and top sessions (table: tokens, cost, account, last active; a session row opens
  its folder or continues it, see `session-handoff-plan.md`).
- Range picker (24h, 7d, 30d, all kept), account filter.
- First open: the import question; the settings live in the section's footer.
- Optional weekly summary notification (off by default).

## Phase 3: Windows app

The same pane in Svelte, charts as plain SVG (no chart library), same first-open question and
settings, same optional weekly toast.

## Tests

- CLI: dedupe by request id; offsets (append, truncate, replace); attribution order with fixtures
  (sessions ledger, profile folder, log timeline, unknown); retention; price lookup by prefix and
  unknown models; report grouping and ranges; JSON contracts; a 50 MB synthetic transcript indexes
  under a few seconds.
- Apps: contract decode; chart series built from fixtures; the import prompt shows once.

## Risks

- Transcripts can be large (GBs on heavy users): the first `all` import is the slow part, so it
  runs in the background with progress, and later runs are incremental.
- Attribution before the ledger exists is approximate (switch log only); the UI says "approximate"
  for those days.
- Claude Code's transcript format is not a public contract: the reader tolerates unknown fields and
  skips lines it cannot read, and a fixture from the current version is in `packages/contracts/`.
- Cost figures go stale between releases; the "as of" label and the CI reminder keep that visible.
