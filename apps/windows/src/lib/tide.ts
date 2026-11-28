// The usage reading ("Tide Staff") rules from DESIGN.md, shared by every surface: pace, fill tier and
// the pace line. Pure functions, tested in tide.test.ts.

import type { Account, Rotation, UsageWindow } from "./types";
import { MAX_JOIN_S, gapSeconds } from "./usage";

export const FIVE_HOUR_S = 5 * 3600;
export const SEVEN_DAY_S = 7 * 24 * 3600;
/** Points from the pace that count as off it; smaller drifts read "on pace". */
export const PACE_THRESHOLD = 5;
/** At or over this a reading turns amber (and almanac figures take colour). */
export const HIGH_PCT = 85;

export type Tier = "unknown" | "live" | "warn" | "limit";

/** The expected pace: the CLI's when it sent one, else linear over the window from the reset time. */
export function expectedPct(w: UsageWindow | null | undefined, windowS: number, now: number): number | null {
  if (!w) return null;
  if (w.expectedPct != null) return w.expectedPct;
  if (!w.resetsAt) return null;
  const left = (Date.parse(w.resetsAt) - now) / 1000;
  if (Number.isNaN(left)) return null;
  const elapsed = 1 - Math.max(0, Math.min(1, left / windowS));
  return elapsed * 100;
}

/** Points ahead (+) or behind (−) the expected pace, when both are known. */
export function paceDelta(w: UsageWindow | null | undefined, windowS: number, now: number): number | null {
  const expected = expectedPct(w, windowS, now);
  if (w?.pct == null || expected == null) return null;
  return Math.round(w.pct - expected);
}

/** Ahead by at least the threshold, else the CLI's own verdict. */
export function isAheadOfPace(w: UsageWindow | null | undefined, windowS: number, now: number): boolean {
  const d = paceDelta(w, windowS, now);
  return d != null ? d >= PACE_THRESHOLD : w?.aheadOfPace === true;
}

/** The Three-Step Fill Rule: live (cyan) on pace, warn (amber) ahead of pace or at 85%+, limit (red) at 100%. */
export function tier(w: UsageWindow | null | undefined, windowS: number, now: number): Tier {
  if (w?.pct == null) return "unknown";
  if (w.pct >= 100) return "limit";
  if (w.pct >= HIGH_PCT || isAheadOfPace(w, windowS, now)) return "warn";
  return "live";
}

export interface PaceLine {
  text: string;
  warn: boolean;
}

/** The details panel's weekly pace: "N pts ahead", "N pts under" or "On pace", by the cards' threshold. */
export function weeklyPace(w: UsageWindow | null | undefined, now: number): PaceLine | null {
  // Only the CLI's expected pace: it leaves it out for the first day after a reset, when any use reads as far ahead.
  if (w?.expectedPct == null) return null;
  const d = paceDelta(w, SEVEN_DAY_S, now);
  const expected = expectedPct(w, SEVEN_DAY_S, now);
  if (d == null || expected == null) return null;
  const head = d >= PACE_THRESHOLD ? `${d} pts ahead` : d <= -PACE_THRESHOLD ? `${-d} pts under` : "On pace";
  return { text: `${head} (expected ${Math.round(expected)}%)`, warn: d >= PACE_THRESHOLD };
}

/** A run-out time on a card: short weekday and 24-hour clock, "Sat 10:15". */
export function runOutLabel(iso: string): string | null {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return null;
  return new Date(t).toLocaleString(undefined, { weekday: "short", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
}

/**
 * A card's weekly pace line, as macOS WeeklyPace: "N pts ahead of pace · runs out Sat 10:15" (the
 * run-out only when the CLI projects the week will not last to its reset), "N pts under pace" or
 * "on pace". Only from the CLI's expected pace, which it leaves out for the first day after a reset.
 */
export function cardWeeklyPace(
  w: UsageWindow | null | undefined,
  now: number,
  label: (iso: string) => string | null = runOutLabel,
): PaceLine | null {
  if (w?.expectedPct == null) return null;
  const d = paceDelta(w, SEVEN_DAY_S, now);
  if (d == null) return null;
  if (d <= -PACE_THRESHOLD) return { text: `${-d} pts under pace`, warn: false };
  if (d < PACE_THRESHOLD) return { text: "on pace", warn: false };
  const runOut = w.willLastToReset === false && w.projectedExhaustionAt ? label(w.projectedExhaustionAt) : null;
  return { text: runOut ? `${d} pts ahead of pace · runs out ${runOut}` : `${d} pts ahead of pace`, warn: true };
}

/** The panel's pace line: "N pts ahead of pace", "N pts under pace", "on pace", or nothing. */
export function paceLine(w: UsageWindow | null | undefined, now: number, windowS: number = FIVE_HOUR_S): PaceLine | null {
  const d = paceDelta(w, windowS, now);
  if (d == null) return null;
  if (d >= PACE_THRESHOLD) return { text: `${d} pts ahead of pace`, warn: true };
  if (d <= -PACE_THRESHOLD) return { text: `${-d} pts under pace`, warn: false };
  return { text: "on pace", warn: false };
}

export function pctLabel(pct: number | null | undefined): string {
  return pct == null ? "–" : `${Math.round(pct)}%`;
}

export function displayName(a: Account): string {
  return a.alias && a.alias.length > 0 ? a.alias : a.email;
}

/** The short name a Switch button uses: the alias, else the email's local part. */
export function shortName(a: Account): string {
  return a.alias && a.alias.length > 0 ? a.alias : (a.email.split("@")[0] ?? a.email);
}

/**
 * An almanac row's name, as macOS `AccountNames.rowName`: the alias, else the email's local part,
 * else (when another account shares that local part) the whole email.
 */
export function rowName(a: Account, accounts: Account[]): string {
  if (a.alias && a.alias.length > 0) return a.alias;
  const local = (e: string) => e.split("@")[0] ?? e;
  const mine = local(a.email).toLowerCase();
  const shared = accounts.some((o) => o.number !== a.number && local(o.email).toLowerCase() === mine);
  return shared ? a.email : local(a.email);
}

/** Below this the week is never flagged as the tighter limit (macOS `Usage.tighterWeekFloor`). */
export const TIGHTER_WEEK_FLOOR = 50;

/**
 * The week has less left than the 5-hour window, past `TIGHTER_WEEK_FLOOR`: the flyout leads with
 * the 5-hour figure and turns the week line amber instead (macOS `Usage.weekIsTighter`).
 */
export function weekIsTighter(usage: Account["usage"]): boolean {
  const week = usage?.sevenDay?.pct;
  if (week == null || week < TIGHTER_WEEK_FLOOR) return false;
  return week > (usage?.fiveHour?.pct ?? 0);
}

/** Whether rows need their plan label: only when the accounts do not all share one plan (macOS `AccountNames.plansVary`). */
export function plansVary(accounts: Account[]): boolean {
  return new Set(accounts.map((a) => a.plan?.label ?? "")).size > 1;
}

export function needsRelogin(a: Account): boolean {
  return ["relogin_required", "no_credentials", "foreign_credential"].includes(a.usageStatus);
}

/** Token and API-key slots (`add-token`) hold no Claude login, so signing in cannot renew them. */
export function canRelogin(a: Account): boolean {
  return a.usageStatus !== "api_key" && !a.email.endsWith("@token.local");
}

/** How long before a login expires that "Sign In Again" is offered (macOS `Account.loginDue`). */
export const LOGIN_DUE_MS = 7 * 86_400_000;

/** Whether the login expires within `LOGIN_DUE_MS` of `now`; false when the expiry is unknown. */
export function loginDue(a: Account, now: number): boolean {
  if (!a.loginExpiresAt) return false;
  const exp = new Date(a.loginExpiresAt).getTime();
  return !Number.isNaN(exp) && exp - now < LOGIN_DUE_MS;
}

/** Whether to offer "Sign In Again" now (macOS `Account.offersSignIn`). */
export function offersSignIn(a: Account, now: number): boolean {
  return canRelogin(a) && (needsRelogin(a) || loginDue(a, now));
}

export function statusNote(a: Account): string | null {
  switch (a.usageStatus) {
    case "ok":
      return null;
    case "token_expired":
      return "Token expired — refreshes automatically";
    case "api_key":
      return "API key (no quota)";
    case "relogin_required":
      return "Re-login needed: use Sign In Again";
    case "foreign_credential":
      return "Credential belongs to another account";
    case "no_credentials":
      return "No stored credentials: use Sign In Again";
    default:
      return "Usage unavailable";
  }
}

/** Active first, then switch order (the CLI's order). */
export function ordered(accounts: Account[]): Account[] {
  return [...accounts].sort((a, b) => Number(b.active) - Number(a.active));
}

/** "Usage fetched 3 min ago", and where, when another of your machines read it. */
export function usageFetchedLabel(a: Account): string | null {
  const age = a.usageAgeSeconds;
  if (age == null || !Number.isFinite(age) || age < 0) return null;
  const minutes = Math.round(age / 60);
  const ago =
    age < 60 ? "just now" : minutes < 60 ? `${minutes} min ago` : `${Math.round(minutes / 60)} h ago`;
  return a.usageFromOtherMachine ? `Usage fetched ${ago}, on another machine` : `Usage fetched ${ago}`;
}

// ---- Week Tide and the rotation verdict (macOS `WeekTide.swift`)

/** A point of the Week Tide in unit coordinates: x 0…1 through the week, y 0…1 of 100%. */
export interface TidePoint {
  x: number;
  y: number;
}

/** One stretch of the curve. `estimated`: not read but assumed, the straight line an average draws. */
export interface TideSegment {
  points: TidePoint[];
  estimated: boolean;
}

/**
 * The curve for one account's week, as macOS `WeekTide.segments`. `readings` are [epoch s, weekly %];
 * only those inside the current week count. The week starts at 0%; the last point is `pct` at `now`
 * (epoch ms). A stretch with no readings (before the first one kept, or a gap) is an estimated line.
 */
export function weekTide(readings: [number, number][], resetsAt: string, pct: number, now: number): TideSegment[] {
  const reset = Date.parse(resetsAt) / 1000;
  if (Number.isNaN(reset)) return [];
  const start = reset - SEVEN_DAY_S;
  const nowS = now / 1000;
  const x = (t: number) => Math.min(1, Math.max(0, (t - start) / SEVEN_DAY_S));
  const y = (p: number) => Math.min(1, Math.max(0, p / 100));
  const inWeek = readings.filter(([t]) => t >= start && t <= nowS).sort((a, b) => a[0] - b[0]);
  const read: [number, number][] = [...inWeek, [nowS, pct]];
  const gap = gapSeconds(read.map(([t]) => t));
  const toPoint = ([t, p]: [number, number]): TidePoint => ({ x: x(t), y: y(p) });

  const out: TideSegment[] = [];
  let current: TidePoint[] = [];
  read.forEach((r, i) => {
    const prev = read[i - 1];
    if (prev && r[0] - prev[0] > gap && current.length) {
      out.push({ points: current, estimated: false });
      out.push({ points: [current.at(-1)!, toPoint(r)], estimated: true });
      current = [];
    }
    current.push(toPoint(r));
  });
  out.push({ points: current, estimated: false });

  const first = read[0]!;
  const lead = first[0] - start;
  if (inWeek.length && lead <= MAX_JOIN_S) out[0] = { points: [{ x: 0, y: 0 }, ...out[0]!.points], estimated: false };
  else out.unshift({ points: [{ x: 0, y: 0 }, toPoint(first)], estimated: true });
  return out.filter((s) => s.points.length > 1);
}

/** "The week lasts" or "Runs dry Sat 10:28". */
export function rotationHeadline(r: Rotation): string {
  const dry = !r.lasts && r.runsDryAt ? runOutLabel(r.runsDryAt) : null;
  return dry ? `Runs dry ${dry}` : "The week lasts";
}

/** The line under the headline, naming the accounts that count and the next reset (macOS `Rotation.detail`). */
export function rotationDetail(r: Rotation): string {
  const count = r.accounts.length === 1 ? "1 account" : `${r.accounts.length} accounts`;
  const reset = r.nextResetAt ? runOutLabel(r.nextResetAt) : null;
  const next = reset ? `the next reset, ${reset}` : "the next reset";
  if (r.lasts || !r.runsDryAt) return `${count} auto-switch can use carry you past ${next}.`;
  const dry = Date.parse(r.runsDryAt);
  const at = r.nextResetAt ? Date.parse(r.nextResetAt) : NaN;
  if (!Number.isNaN(at) && dry < at) {
    const hours = Math.round((at - dry) / 3_600_000);
    return `${hours} h before ${next}, across ${count} auto-switch can use.`;
  }
  return `Across ${count} auto-switch can use, at the pace of the last few days.`;
}

/**
 * Cut a long name in the middle, keeping both ends: "Dep1pj.claude02@…system.com". Accounts on one
 * domain differ near the middle, so an end cut would hide the part that tells them apart. Cards have a
 * fixed width, so a character budget fits (macOS: `.truncationMode(.middle)`).
 */
export function middleEllipsis(text: string, max: number): string {
  if (text.length <= max) return text;
  const keep = max - 1;
  const head = Math.ceil(keep * 0.65);
  return `${text.slice(0, head)}…${text.slice(text.length - (keep - head))}`;
}
