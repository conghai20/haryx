// The usage reading ("Tide Staff") rules from DESIGN.md, shared by every surface: pace, fill tier and
// the pace line. Pure functions, tested in tide.test.ts.

import type { Account, UsageWindow } from "./types";

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

/** The panel's pace line: "N pts ahead of pace", "N pts under pace", "on pace", or nothing. */
export function paceLine(w: UsageWindow | null | undefined, now: number): PaceLine | null {
  const d = paceDelta(w, FIVE_HOUR_S, now);
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

export function needsRelogin(a: Account): boolean {
  return ["relogin_required", "no_credentials", "foreign_credential"].includes(a.usageStatus);
}

/** Token and API-key slots (`add-token`) hold no Claude login, so signing in cannot renew them. */
export function canRelogin(a: Account): boolean {
  return a.usageStatus !== "api_key" && !a.email.endsWith("@token.local");
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
