# Usage share between machines — plan

Status: built on all three surfaces (2026-11-27); the server migration is not deployed yet.

## Problem

The Claude usage endpoint (`/api/oauth/usage`) gives each account a budget of about 28-30
requests in a trailing hour (`cli/crates/haryx-core/src/poll_policy.rs`). The budget belongs to the
account, not to the machine. HaryX sync carries accounts, settings and tokens between a user's
machines, but never usage (`sync/sources.rs`: "Never a token, never usage"), so every machine runs
its own poll schedule for every account:

- one machine polls an active account at most every 180 s (20/hour), safe on its own;
- two machines with the same account active poll it 40 times an hour, over the cap;
- a 429 seen on one machine is unknown to the other, which keeps probing inside the block, and a
  probe inside a block was measured to extend it by up to an hour.

The only defence today is reactive: after 429s each machine widens its own interval (AIMD, x1.5 up
to 30 min). It converges, but only after the 429s that cause the delay users see.

## Goal

All of a user's machines share **one poll schedule per account**: one budget, one 429 state, one
reading. Two machines together send what one machine sends today, and a hard ledger cap holds even
when the schedule misbehaves.

Health check after the change: zero `http-429` at steady state with two machines.

## Design

### 1. A shared `usage` row per account

A new sync row kind, `usage`, one row per account. Key: `SyncKey::row_key("usage:<email>:<org>")`
(HMAC, as other rows). Body: the usual sealed `Document`; its values:

| Field | Meaning |
|---|---|
| `email`, `org` | identity (inside the encryption only) |
| `lastGood`, `fetchedAt`, `fetchedBy` | the newest reading, when (server time) and which machine |
| `nextPollAt`, `pollIntervalS` | the shared plan |
| `backoffUntil`, `last429At` | the shared failure state |
| `claimUntil`, `claimBy` | the lease of the machine fetching now |
| `requests` | `[[time, machine], ...]` of the trailing hour: the shared ledger |
| `activeOn` | `{machine: until}`: machines where this account is the active one |

Usage rows bypass the three-way-merge sync pass (`sync::run` skips the kind). They are read and
written directly by the usage collector, last writer wins by `fetchedAt`, with the row `version`
as compare-and-set. Tokens are never in them.

Old HaryX versions list the rows, open them, find no local item for the kind and leave them alone.
The server's `kind` check needs a migration (`server/supabase/migrations/…_sync_rows_usage.sql`);
until it is deployed, writes fail and the collector falls back (section 6).

### 2. Collection flow

`Switcher::collect_usage_entries` today: reserve (local lease) → fetch → record. With the share:

1. **Pull**, only when some requested account is locally eligible to fetch (so a dashboard refresh
   that would fetch nothing costs nothing): one `GET sync_rows?kind=eq.usage`. Each row that opens
   is merged into the local store (`UsageStore::merge_peer`):
   - a newer reading replaces `lastGood`/`fetchedAt` **and** the plan (the schedule is shared);
   - `backoffUntil` and `last429At` take the later of the two;
   - another machine's live lease becomes a local `heldUntil` (no fetch while it is in flight);
   - other machines' ledger entries are merged into the local ledger;
   - `activeOn` of other machines becomes `activeElsewhereUntil`.
2. **Reserve** locally as today; eligibility now also requires the ledger to be under the cap.
3. **Claim** each reserved account on the server: CAS-update its row with this machine's lease and
   a ledger entry. Lost (version moved, another live lease, or a fresh reading arrived) → release
   the local lease and do not fetch.
4. **Fetch** and **record** locally as today.
5. **Push** each recorded outcome: reading, plan, failure state, ledger, and this machine's
   `activeOn` mark; the lease is cleared. A CAS conflict re-reads and retries once.

Cost per machine: one GET per pass that would fetch, plus two PATCHes per actual fetch, to the sync
server. Nothing extra to Anthropic.

### 3. One 429, every machine

`backoffUntil` and `last429At` travel in the row, so a 429 on one machine parks the account on
every machine for the same deadline. The other machine never probes inside the block.

### 4. Hourly ledger cap (defence in depth)

`HOURLY_REQUEST_CAP = 24` fetches per account per trailing hour, counting every machine, out of the
measured ~28-30: the rest is left for clients HaryX cannot coordinate (Claude Code's `/usage`,
statusline tools). The ledger lives in the local row too, so the cap holds on a single machine and
during long urgent-mode episodes.

### 5. Active anywhere counts as active

The planner treats an account as active when this machine or any other one uses it
(`activeElsewhereUntil`, a 15 min mark refreshed on every push). Whichever machine fetches, the
account keeps the active cadence and urgent mode.

### 6. Fallback without coordination

Share off, signed out, sync locked, offline, or the server refusing the kind: collect exactly as
today, except that when the local ledger saw other machines fetch an account in the last hour, its
planned interval is multiplied by `1 + other machines`. Two uncoordinated machines then each poll
half as often.

### 7. Clocks

Shared times are server times. Each pull reads the server clock from the response's `Date` header
and converts with that offset, so a machine with a wrong clock still agrees on deadlines.

## Setting

`usage.shareBetweenMachines` (bool, default on). It only acts while this machine is signed in to
HaryX with sync unlocked. Both apps list every `haryx config` key in their settings, so the switch
appears in each without new UI; the macOS list adds its one-line help.

## Surfaces

| Surface | Change |
|---|---|
| CLI | all of the above in `haryx-core` (`usage_share/`, `usage_store/peer.rs`, `poll_policy`, `sync`), the setting, `list --json` gains `fetchedBy: "peer"` on readings another machine took |
| macOS app | the setting in the config list (with help); the inspector's "Usage fetched" adds ", on another machine" |
| Windows app | the setting in the config list; the inspector gains the "Usage fetched … ago" caption with the same suffix |

## Tests

Against the in-memory `sync::remote::Memory`:

- a fresher peer reading is adopted and no fetch happens;
- two collectors over one server: the second loses the CAS claim and does not fetch;
- a 429 recorded by one machine parks the other;
- the ledger cap blocks a fetch even when the plan says due;
- `activeOn` from a peer gives the active cadence;
- unreachable server: the fallback multiplies the interval by the peers seen;
- clock offset from the `Date` header converts deadlines both ways.

## Rollout

1. CLI + migration (this change). The migration must be deployed before the share does anything;
   until then the fallback applies.
2. Apps: setting and label.
3. Watch logs on two machines for `http-429`.
