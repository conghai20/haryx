// Continue a session on another account (docs/session-handoff-plan.md): which sessions are stuck,
// and which account "best" means. The CLI does the work (`ai sessions continue`); these only choose.

import { sharesGroup } from "./groups";
import { needsRelogin } from "./tide";
import type { Account, Session } from "./types";

/** The higher of the 5-hour and weekly %, or null when unknown. */
export function peakPct(a: Account): number | null {
  const pcts = [a.usage?.fiveHour?.pct, a.usage?.sevenDay?.pct].filter((p): p is number => p != null);
  return pcts.length ? Math.max(...pcts) : null;
}

export function exhausted(a: Account | undefined): boolean {
  return a != null && (peakPct(a) ?? 0) >= 100;
}

/**
 * Out of use for now, held out of rotation or at its limit: dimmed, with a sleeping mascot (macOS
 * `Account.isResting`). An account at its limit stays in rotation and wakes when its quota resets.
 */
export const resting = (a: Account) => !!a.disabled || exhausted(a);

export interface Stuck {
  session: Session;
  /** The account it ran out on. */
  account: number;
}

/**
 * Sessions to offer a handoff for: `haryx run` sessions on an exhausted account. Those run in their
 * account's own profile, so a switch does not move them. A default-profile session is never stuck
 * this way: it picks up a switched account while it runs, so switching is its fix.
 */
export function stuckSessions(sessions: Session[], accounts: Account[]): Stuck[] {
  const byNumber = new Map(accounts.map((a) => [a.number, a]));
  return sessions.flatMap((s) =>
    s.profile !== "default" && s.account != null && exhausted(byNumber.get(s.account))
      ? [{ session: s, account: s.account }]
      : [],
  );
}

/**
 * Where a session that ran out on `from` should continue: a usable account with headroom, other than
 * `from`, with the most headroom. One in `from`'s group wins over any outside it; only when none
 * there can take it does the pick leave the group, as the CLI's `--to best` does. Mirrors macOS
 * `AccountGroups.handoffTarget`.
 */
export function bestTarget(accounts: Account[], from: number | null): Account | null {
  const usable = accounts.filter(
    (a) => a.number !== from && !a.disabled && !needsRelogin(a) && (peakPct(a) ?? 100) < 100,
  );
  const source = accounts.find((a) => a.number === from);
  const mates = source ? usable.filter((a) => sharesGroup(a, source)) : [];
  return (mates.length > 0 ? mates : usable).reduce<Account | null>(
    (best, a) => (best == null || (peakPct(a) ?? 100) < (peakPct(best) ?? 100) ? a : best),
    null,
  );
}
