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

export function shortTokens(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}k`;
  return String(n);
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

/** SVG polyline points for one account's window over time, x scaled across [from, to]. */
export function quotaLine(
  readings: Reading[],
  account: number,
  window: "fiveHour" | "sevenDay",
  from: number,
  to: number,
  width: number,
  height: number,
): string {
  const span = Math.max(1, to - from);
  return readings
    .filter((r) => r.account === account && r[window] != null)
    .map((r) => {
      const x = ((r.at - from) / span) * width;
      const y = height - (Math.min(105, r[window] as number) / 105) * height;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}

/** A fixed, readable colour per account number. */
export function accountColor(n: number): string {
  const palette = ["#3b82c4", "#d9822b", "#2e9e6b", "#b84a9e", "#7a5af5", "#c9a227", "#d64545", "#3fa7a7"];
  return palette[(((n - 1) % palette.length) + palette.length) % palette.length] ?? "#888888";
}
