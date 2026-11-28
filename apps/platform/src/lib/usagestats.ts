// Usage as the web shows it: per account per day, every machine's rows added together, tokens by
// model, and the day's highest 5h/7d readings. Bodies carry no prices, so this counts tokens only.
import type { UsageSync, UsageSyncDay } from "./types";
import type { SyncedAccount } from "./synced";

export interface ModelTokens {
  model: string;
  replies: number;
  input: number;
  output: number;
  cacheRead: number;
  /** Both cache-write lifetimes. */
  cacheWrite: number;
  total: number;
}

export interface DayUsage {
  day: string;
  total: number;
  replies: number;
  /** Highest total first. */
  models: ModelTokens[];
  /** The day's highest 5h and 7d readings (local day of the reading), null when none. */
  fiveHourPct: number | null;
  sevenDayPct: number | null;
}

export interface Reading {
  at: number;
  fiveHourPct: number | null;
  sevenDayPct: number | null;
}

export interface AccountUsage {
  /** Lowercased email, or null for usage no account was attributed to. */
  account: string | null;
  total: number;
  /** Newest first. */
  days: DayUsage[];
  /** The newest reading across machines. */
  latest: Reading | null;
  /** Machines whose rows carry this account's tokens. */
  machineIds: string[];
}

export const normEmail = (email: string | null | undefined) => (email ? email.trim().toLowerCase() : null);

const tokensOf = (d: UsageSyncDay) => d.input + d.output + d.cacheRead + d.cacheWrite5m + d.cacheWrite1h;

/** `yyyy-mm-dd` of epoch seconds in the browser's time zone. */
export function localDay(at: number): string {
  const d = new Date(at * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Months the bodies cover, newest first. */
export const monthsOf = (bodies: readonly UsageSync[]) => [...new Set(bodies.map((b) => b.month))].sort().reverse();

const maxOf = (a: number | null, b: number | null) => (a === null ? b : b === null ? a : Math.max(a, b));

interface DayDraft {
  models: Map<string, ModelTokens>;
  fiveHourPct: number | null;
  sevenDayPct: number | null;
}

interface AccountDraft {
  days: Map<string, DayDraft>;
  latest: Reading | null;
  machines: Set<string>;
}

const newDay = (): DayDraft => ({ models: new Map(), fiveHourPct: null, sevenDayPct: null });

function draftFor(map: Map<string | null, AccountDraft>, account: string | null): AccountDraft {
  let draft = map.get(account);
  if (!draft) {
    draft = { days: new Map(), latest: null, machines: new Set() };
    map.set(account, draft);
  }
  return draft;
}

function dayFor(draft: AccountDraft, day: string): DayDraft {
  let d = draft.days.get(day);
  if (!d) {
    d = newDay();
    draft.days.set(day, d);
  }
  return d;
}

function addDay(draft: AccountDraft, row: UsageSyncDay) {
  const models = dayFor(draft, row.day).models;
  const m = models.get(row.model) ?? { model: row.model, replies: 0, input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 };
  models.set(row.model, {
    model: row.model,
    replies: m.replies + row.replies,
    input: m.input + row.input,
    output: m.output + row.output,
    cacheRead: m.cacheRead + row.cacheRead,
    cacheWrite: m.cacheWrite + row.cacheWrite5m + row.cacheWrite1h,
    total: m.total + tokensOf(row),
  });
}

function finishDay(day: string, d: DayDraft): DayUsage {
  const models = [...d.models.values()].sort((a, b) => b.total - a.total || a.model.localeCompare(b.model));
  return {
    day,
    total: models.reduce((n, m) => n + m.total, 0),
    replies: models.reduce((n, m) => n + m.replies, 0),
    models,
    fiveHourPct: d.fiveHourPct,
    sevenDayPct: d.sevenDayPct,
  };
}

/**
 * The bodies of `month` added up per account and day across machines. Quota readings count on the
 * day they were taken, in the browser's time zone; a reading taken outside `month` is left out.
 * Accounts with the most tokens come first; unattributed usage comes last.
 */
export function aggregate(bodies: readonly UsageSync[], month: string): AccountUsage[] {
  const map = new Map<string | null, AccountDraft>();
  for (const body of bodies) {
    if (body.month !== month) continue;
    for (const row of body.days) {
      const draft = draftFor(map, normEmail(row.account));
      addDay(draft, row);
      draft.machines.add(body.machineId);
    }
    for (const q of body.quota) {
      const day = localDay(q.at);
      if (!day.startsWith(month)) continue;
      const draft = draftFor(map, normEmail(q.account));
      const d = dayFor(draft, day);
      d.fiveHourPct = maxOf(d.fiveHourPct, q.fiveHourPct);
      d.sevenDayPct = maxOf(d.sevenDayPct, q.sevenDayPct);
      if (!draft.latest || q.at > draft.latest.at) draft.latest = { at: q.at, fiveHourPct: q.fiveHourPct, sevenDayPct: q.sevenDayPct };
    }
  }
  const out: AccountUsage[] = [...map.entries()].map(([account, draft]) => {
    const days = [...draft.days.entries()].map(([day, d]) => finishDay(day, d)).sort((a, b) => b.day.localeCompare(a.day));
    return { account, total: days.reduce((n, d) => n + d.total, 0), days, latest: draft.latest, machineIds: [...draft.machines].sort() };
  });
  return out.sort((a, b) => (a.account === null ? 1 : 0) - (b.account === null ? 1 : 0) || b.total - a.total || (a.account ?? "").localeCompare(b.account ?? ""));
}

/** Every machine id whose usage names each account (lowercased email), over all months. */
export function machinesByAccount(bodies: readonly UsageSync[]): Map<string, string[]> {
  const map = new Map<string, Set<string>>();
  for (const body of bodies)
    for (const row of body.days) {
      const email = normEmail(row.account);
      if (!email) continue;
      const set = map.get(email) ?? new Set<string>();
      set.add(body.machineId);
      map.set(email, set);
    }
  return new Map([...map].map(([email, set]) => [email, [...set].sort()]));
}

/** What to call an account: its alias, else its email (several orgs' aliases joined). */
export function accountTitle(accounts: readonly SyncedAccount[], email: string | null): string {
  if (email === null) return "Not attributed to an account";
  const aliases = [...new Set(accounts.filter((a) => normEmail(a.email) === email && a.alias).map((a) => a.alias!))];
  return aliases.length ? aliases.join(" · ") : email;
}

/** A percentage reading, or a dash. */
export const pct = (v: number | null) => (v === null ? "—" : `${Math.round(v)}%`);
