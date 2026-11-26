// Continue a session on another account (docs/session-handoff-plan.md): which sessions are stuck,
// and which account "best" means. The CLI does the work (`ai sessions continue`); these only choose.

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
 * Sessions to offer a handoff for: an own-profile session on an exhausted account, and a
 * default-profile session on an exhausted account, where `leftBehind` (pid → account) says which
 * account a default-profile session kept after an auto-switch.
 */
export function stuckSessions(
  sessions: Session[],
  accounts: Account[],
  leftBehind: Record<number, number>,
): Stuck[] {
  const byNumber = new Map(accounts.map((a) => [a.number, a]));
  return sessions.flatMap((s) => {
    const ranOn = s.profile === "default" ? (leftBehind[s.pid] ?? s.account) : s.account;
    return ranOn != null && exhausted(byNumber.get(ranOn)) ? [{ session: s, account: ranOn }] : [];
  });
}

/** The usable account with the most headroom, other than `from`. */
export function bestTarget(accounts: Account[], from: number | null): Account | null {
  const usable = accounts.filter(
    (a) => a.number !== from && !a.disabled && !needsRelogin(a) && (peakPct(a) ?? 100) < 100,
  );
  return usable.reduce<Account | null>(
    (best, a) => (best == null || (peakPct(a) ?? 100) < (peakPct(best) ?? 100) ? a : best),
    null,
  );
}
