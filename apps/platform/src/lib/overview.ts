// The account overview the web opens on: every synced account with its newest quota reading, the
// machine that took it, and its last seven days of weekly readings. Synced readings are history,
// never live: the web says when and where each one was read and claims nothing about "now".
import type { SyncedAccount, Synced } from "./synced";
import { normEmail } from "./usagestats";
import { relativeTime } from "@win/lib/text";
import { resolve as resolveMascots, type Style as MascotStyle } from "@win/lib/mascot";

export interface AccountReading {
  /** Unix seconds. */
  at: number;
  machineId: string;
  fiveHourPct: number | null;
  sevenDayPct: number | null;
}

export interface WeekPoint {
  at: number;
  pct: number;
}

export interface AccountOverview {
  /** `email|organizationUuid`, unique per synced account. */
  id: string;
  account: SyncedAccount;
  /** Alias, else email. */
  title: string;
  /** The newest reading of this account's email on any machine, or null. */
  reading: AccountReading | null;
  /** 7-day readings taken in the seven days before `now`, oldest first. */
  week: WeekPoint[];
  /** Machines whose usage names this account, sorted. */
  machineIds: string[];
  /** The account's mascot, as the apps chose it (`app-prefs`), else one assigned the same way. */
  mascot: MascotStyle;
}

export interface Overview {
  accounts: AccountOverview[];
  /** The account read most recently, when any account has a reading. */
  lastUsed: AccountOverview | null;
}

export const WEEK_S = 7 * 86_400;

/** Which fill a reading gets: the normal one, amber from 85%, red at 100% (Three-Step Fill Rule). */
export type Fill = "normal" | "amber" | "red";
export function fillOf(pct: number | null): Fill {
  if (pct === null) return "normal";
  if (pct >= 100) return "red";
  if (pct >= 85) return "amber";
  return "normal";
}

/** A percentage clamped to the 0-100 a staff can draw. */
export const clampPct = (pct: number | null) => (pct === null ? 0 : Math.min(100, Math.max(0, pct)));

export const accountId = (a: SyncedAccount) => `${a.email}|${a.organizationUuid}`;

export function overview(synced: Synced, now: number): Overview {
  const latest = new Map<string, AccountReading>();
  const week = new Map<string, WeekPoint[]>();
  const machines = new Map<string, Set<string>>();
  const since = now - WEEK_S;

  for (const body of synced.usage) {
    for (const row of body.days) {
      const email = normEmail(row.account);
      if (!email) continue;
      const set = machines.get(email) ?? new Set<string>();
      set.add(body.machineId);
      machines.set(email, set);
    }
    for (const q of body.quota) {
      const email = normEmail(q.account);
      if (!email) continue;
      const prev = latest.get(email);
      if (!prev || q.at > prev.at) {
        latest.set(email, { at: q.at, machineId: body.machineId, fiveHourPct: q.fiveHourPct, sevenDayPct: q.sevenDayPct });
      }
      if (q.sevenDayPct !== null && q.at >= since && q.at <= now) {
        const points = week.get(email) ?? [];
        points.push({ at: q.at, pct: q.sevenDayPct });
        week.set(email, points);
      }
    }
  }

  const mascots = resolveMascots(synced.mascotStyles, synced.accounts.map(accountId));
  const accounts = synced.accounts.map((account): AccountOverview => {
    const email = normEmail(account.email) ?? "";
    return {
      id: accountId(account),
      account,
      title: account.alias ?? account.email,
      reading: latest.get(email) ?? null,
      week: (week.get(email) ?? []).sort((a, b) => a.at - b.at),
      machineIds: [...(machines.get(email) ?? [])].sort(),
      mascot: mascots[accountId(account)]!,
    };
  });

  let lastUsed: AccountOverview | null = null;
  for (const a of accounts) {
    if (a.reading && (!lastUsed || a.reading.at > lastUsed.reading!.at)) lastUsed = a;
  }
  return { accounts, lastUsed };
}

/** "2 min ago", "3 hr ago", "2 days ago": the same wording as Machines (`lastSeen`). */
export const ago = (at: number, now: number): string => relativeTime(at * 1000, now * 1000);

/** An email split for display: the part before "@" leads, "@domain" follows. Null for a non-email. */
export function splitEmail(title: string): { local: string; domain: string } | null {
  const at = title.lastIndexOf("@");
  return at > 0 && !title.includes(" ") ? { local: title.slice(0, at), domain: title.slice(at) } : null;
}

/** Text cut before ".", "-", "_" and "@", so a long address breaks (and truncates) between words, never after a dot. */
export const breakable = (text: string): string[] => text.split(/(?=[.\-_@])/);

/**
 * The Week Tide's path over the seven days before `now`, in a `width` x `height` box: the area
 * under the readings and its top line. Null with fewer than two points.
 */
export function weekPath(points: readonly WeekPoint[], now: number, width: number, height: number): { area: string; line: string } | null {
  if (points.length < 2) return null;
  const x = (at: number) => ((at - (now - WEEK_S)) / WEEK_S) * width;
  const y = (pct: number) => height - (clampPct(pct) / 100) * height;
  const coords = points.map((p) => `${x(p.at).toFixed(1)} ${y(p.pct).toFixed(1)}`);
  const line = `M${coords.join(" L")}`;
  const first = x(points[0]!.at).toFixed(1);
  const last = x(points[points.length - 1]!.at).toFixed(1);
  return { line, area: `${line} L${last} ${height} L${first} ${height} Z` };
}
