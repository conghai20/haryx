// `haryx ai usage … --json` (contracts: packages/contracts/haryx/usage-*.json) and the plain-SVG
// chart geometry the Usage pane draws.

export interface HistoryStatus {
  enabled: boolean;
  needsChoice: boolean;
  readReplies: number;
  keepDays: number;
}

export interface UsageRow {
  key: string | null;
  replies: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  tokens: number;
  cost: number | null;
  unpricedTokens: number;
  share: number;
  approximate: boolean;
  lastActive: number | null;
  accounts: number[];
}

export interface UsageReport {
  by: string;
  since: number;
  pricesAsOf: string;
  rows: UsageRow[];
  total: UsageRow;
  history?: HistoryStatus;
}

export interface Reading {
  at: number;
  account: number;
  fiveHour: number | null;
  sevenDay: number | null;
}

export interface QuotaReport {
  since: number;
  readings: Reading[];
  history?: HistoryStatus;
}

/** A count in a few characters: 812, 35K, 12.3M, 400M, 1.2B; a trailing ".0" is dropped (macOS `UsageChartSupport.compact`). */
export function shortTokens(n: number): string {
  const units: [number, string][] = [
    [1e9, "B"],
    [1e6, "M"],
    [1e3, "K"],
  ];
  const [size, suffix] = units.find(([u]) => Math.abs(n) >= u) ?? [1, ""];
  return `${(n / size).toFixed(1).replace(/\.0$/, "")}${suffix}`;
}

export function money(cost: number | null): string {
  return cost == null ? "—" : `$${cost.toFixed(2)}`;
}

export interface Bar {
  day: string;
  x: number;
  width: number;
  segments: { account: number; y: number; height: number; value: number }[];
}

/**
 * Stacked bars, one per day, from per-account day reports (`report --by day --account N`), scaled
 * into a `width` x `height` box. `cost` picks dollars instead of tokens.
 */
export function stackedBars(
  byAccount: Record<number, UsageReport>,
  cost: boolean,
  width: number,
  height: number,
): { bars: Bar[]; max: number } {
  const days = new Map<string, Map<number, number>>();
  for (const [account, report] of Object.entries(byAccount)) {
    for (const row of report.rows) {
      if (!row.key) continue;
      const value = cost ? (row.cost ?? 0) : row.tokens;
      const day = days.get(row.key) ?? new Map<number, number>();
      day.set(Number(account), (day.get(Number(account)) ?? 0) + value);
      days.set(row.key, day);
    }
  }
  const ordered = [...days.keys()].sort();
  const totals = ordered.map((d) => [...days.get(d)!.values()].reduce((a, b) => a + b, 0));
  const max = Math.max(0, ...totals);
  const slot = ordered.length ? width / ordered.length : 0;
  const bars = ordered.map((day, i) => {
    let top = height;
    const segments = [...days.get(day)!.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([account, value]) => {
        const h = max > 0 ? (value / max) * height : 0;
        top -= h;
        return { account, y: top, height: h, value };
      });
    return { day, x: i * slot + slot * 0.15, width: slot * 0.7, segments };
  });
  return { bars, max };
}

/** Never break between readings closer than this, and never join readings further apart than MAX_JOIN_S. */
export const MIN_BREAK_S = 15 * 60;
export const MAX_JOIN_S = 2 * 60 * 60;

/** The longest step a line may span: 3x the median spacing, kept within [15 min, 2 h] (macOS `UsageChartSupport.segments`). */
export function gapSeconds(times: number[]): number {
  const steps = times.slice(1).map((t, i) => t - times[i]!);
  if (steps.length === 0) return MAX_JOIN_S;
  const median = [...steps].sort((a, b) => a - b)[Math.floor(steps.length / 2)]!;
  return Math.min(MAX_JOIN_S, Math.max(MIN_BREAK_S, 3 * median));
}

/**
 * Account colours by account number, one per account in every chart and table and on macOS
 * (`UsageChartSupport.palette`). None is a reading colour: no cyan (live), amber or yellow (pace), red
 * (limit), or a lime that reads as either. `ACCOUNT_PALETTE` is for dark backgrounds, `ACCOUNT_PALETTE_LIGHT`
 * (same hues, deeper) for light ones.
 */
export const ACCOUNT_PALETTE = ["#A78BFA", "#34D399", "#F472B6", "#60A5FA", "#D6B98C", "#6366F1", "#A8A29E", "#9CA36B"];
export const ACCOUNT_PALETTE_LIGHT = ["#7C3AED", "#059669", "#DB2777", "#2563EB", "#8B6F47", "#4F46E5", "#78716C", "#65733A"];

export function accountColor(n: number, light = false): string {
  const list = light ? ACCOUNT_PALETTE_LIGHT : ACCOUNT_PALETTE;
  const len = list.length;
  return list[(((n - 1) % len) + len) % len]!;
}

/** An account's colour as a CSS value that follows the light or dark theme. */
export function accountFill(n: number): string {
  return `light-dark(${accountColor(n, true)}, ${accountColor(n)})`;
}
