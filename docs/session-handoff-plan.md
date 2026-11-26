# Continue a session on another account: plan

Status: built (2026-11-24) without the phase 0 spike, which needs an account at its limit. Tested
with unit and CLI tests; not yet run against a real limit.

**Phase 0, question 1, answered by the owner on 2026-11-26: yes.** A running default-profile
session picks up a switched account while it runs, by hand or by auto-switch; no stop and resume
is needed. Changed to match:
- Only a `haryx run` session (its own profile, tied to one account) is offered a handoff when its
  account is exhausted. Neither app tracks "left behind" default-profile sessions any more, and
  macOS no longer posts "N sessions stopped on account A" after an auto-switch.
- The CLI treats both kinds alike: `--to best` leaves out the account the session runs as, and
  continuing a session on that same account is refused (for a default-profile session it would
  only restart it).
- `best` prefers the source account's group (`haryx group`), as the automatic switches do, and
  leaves it only when no account there has headroom.

The bullets below record the build as it first was; the "left behind" ones no longer hold.

Built as planned, with these differences:
- No `stuck` flag in `ai sessions --json`: computing headroom on every 4-second scan would read the
  usage store each time. The apps decide "stuck" from the account readings they already have.
- A default-profile session is listed under the *active* account, so after an auto-switch it would
  look healthy. Each app remembers, at the switch, which default-profile sessions were running on
  the old account ("left behind") and treats those as stuck when that account is exhausted. The
  CLI allows continuing a default-profile session on the active account (that restarts it on the
  new login); a `haryx run` session must move to another account.
- Windows: the CLI asks the user to close the old window and waits up to 5 minutes for it (no
  graceful signal reaches a console app from outside), instead of failing.

Decisions taken with the owner on 2026-11-24:
- "Continue on account N" **ends the old session itself** (a normal termination request, never a
  hard kill), waits for it, then resumes the same conversation on the new account.
- When auto-switch leaves an exhausted account, HaryX **offers** to continue that account's
  running sessions (notification + menu bar/flyout row, one click). Nothing moves on its own.
  *Superseded 2026-11-26: default-profile sessions follow the switch by themselves; only
  `haryx run` sessions are offered a handoff.*

## The problem

When an account hits its 5-hour or weekly limit, the running Claude Code session stops ("limit
reached"). Auto-switch only changes the account for *new* sessions; the stuck one stays stuck, and
the user has to find its folder, close it, switch, and remember `claude --resume`.

## What already exists

- Claude Code writes `<profile>/sessions/{pid}.json` with `pid`, `sessionId`, `cwd`, `status`, and
  HaryX reads them for every profile (`running.rs`, `haryx ai sessions --json`): which account each
  live session runs as (default profile = the active account; a `haryx run` profile = its slot).
- Conversation history is shared by default: a `haryx run` profile and `~/.claude` see the same
  transcripts (`session_manager.rs`, `session_history.rs`), so `--resume <id>` works across accounts.
- `haryx run N -- <claude args>` launches Claude as account N in this terminal only.
- Both apps open terminals in a folder running a command (macOS `TerminalLauncher`, Windows
  `terminals.rs`), and both track auto-switch events.

## Phase 0: spike (half a day, before any code)

Three facts decide details; each is checked by hand and written down here:
1. **Does a running default-profile session pick up switched credentials?** Hit a limit, switch
   with `haryx switch`, type "continue" in the old session. If it works, a default-profile session
   after an auto-switch needs no handoff, only a hint ("type continue"); the handoff is then for
   `haryx run` sessions and for moving to a non-active account.
2. **Does `claude --resume <id>` load a conversation another process just left** (its
   `sessions/{pid}.json` gone, transcript complete)? And what does it cost (the cache is cold)?
3. **Windows: how to end a console Claude gracefully.** `taskkill` without `/F` does not reach a
   console app; try `GenerateConsoleCtrlEvent(CTRL_BREAK)` via `AttachConsole(pid)` from a helper
   (`haryxw`). If nothing graceful works, Windows asks the user to close it instead.

## Phase 1: CLI

`haryx ai sessions continue <PID|SESSION_ID> [--to NUM|EMAIL|best] [--json] [--no-launch]`

1. Find the live session (by pid or session id) in the scan; refuse if it is not found.
2. Pick the target: `--to`, else `best` (the `switch --strategy best` reading, excluding the
   session's own account, skipping disabled and sign-in-needed accounts). No usable account: say so.
3. End the old process: SIGTERM (Unix) / the Windows mechanism from phase 0. Wait up to 10 s for
   the pid to exit and its `sessions/{pid}.json` to go. Still running: stop with "close it, then
   run this again" and change nothing.
4. Make the transcript reachable from the target profile: find `<sessionId>.jsonl` under the source
   profile's `projects/`; when the target profile does not share history (`--no-share` runs), copy
   it (never move) into the target's `projects/<slug>/`.
5. Resume, in this terminal, in the session's `cwd`:
   - target is the active account: `claude --resume <id>` (default profile);
   - otherwise: what `haryx run <target> -- --resume <id>` does (its own profile; the default login
     is untouched).
   `--no-launch` stops after step 4 and prints the command (for the apps' own terminals).
6. Log one line to `haryx.log` (`Continued session <id> from account X on account Y`), which the
   usage history (`usage-history-plan.md`) also reads for attribution.

`--json` prints `{sessionId, from, to, cwd, command}`; contract
`packages/contracts/haryx/session-continue.json`.

Also: `haryx ai sessions --json` gains `stuck: true` for a session whose account is exhausted
(5-hour or weekly at its limit, from the usage store), so the apps need no rule of their own.

## Phase 2: apps

- **Sessions pane** (both apps): each session gets "Continue on…" (a menu: best, then each usable
  account with its headroom). Stuck sessions are marked.
- **Menu bar panel / tray flyout**: a row when sessions are stuck: "2 sessions stopped on account 1
  — Continue on account 3", one click continues all of them.
- **After auto-switch** leaves an exhausted account with live sessions: a notification ("Continue 2
  sessions on account 3?") with a Continue action; the same row as above.
- The app opens its terminal in the session's folder running `haryx ai sessions continue <pid> --to
  N`, so the CLI does the ending, copying and resuming, and the app only chooses where.
- Notifications category "Sessions moved" (on by default), like the others.

## Tests

- CLI: target choice (best, explicit, none usable); refuses an unknown pid; waits and gives up
  cleanly when the process will not exit (a test child that ignores SIGTERM); transcript copy
  between profiles (never a move, existing file left alone); `--no-launch` output; `stuck` flag
  from usage fixtures; JSON contract.
- macOS/Windows: contract decode; the row and notification appear for stuck sessions (view-model
  tests); terminal command built correctly.
- Manual (both OSes): a real limit, continue, conversation intact, old process gone.

## Risks

- Ending a session mid-edit: Claude only stops at a limit between turns, and SIGTERM lets it save;
  HaryX only offers "continue" for sessions whose account is exhausted, unless the user picks it.
- A resumed conversation re-reads its context without cache: the first turn on the new account
  costs more. Shown in the confirmation the first time.
- Session ids and folders come from Claude Code's own files; a future Claude Code that changes
  `sessions/{pid}.json` breaks this the same way it would break `ai sessions` (contract-tested).
