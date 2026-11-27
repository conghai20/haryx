import { describe, expect, it } from "vitest";
import reportJson from "../../../../packages/contracts/haryx/usage-report.json?raw";
import quotaJson from "../../../../packages/contracts/haryx/usage-quota.json?raw";
import { ACCOUNT_PALETTE, ACCOUNT_PALETTE_LIGHT, accountColor, accountFill, gapSeconds, money, shortTokens, stackedBars, type QuotaReport, type UsageReport } from "./usage";


const day = (rows: [string, number, number | null][]): UsageReport => ({
  by: "day",
  since: 0,
  pricesAsOf: "2026-11-03",
  rows: rows.map(([key, tokens, cost]) => ({
    key, tokens, cost, replies: 1, input: 0, output: 0, cacheRead: 0, cacheWrite: 0,
    unpricedTokens: 0, share: 0, approximate: false, lastActive: null, accounts: [],
  })),
  total: {} as never,
});

describe("usage", () => {
  it("reads the contracts", () => {
    const report = JSON.parse(reportJson) as UsageReport;
    expect(report.rows[0]?.cost).toBe(157.64);
    expect(report.history?.keepDays).toBe(90);
    const quota = JSON.parse(quotaJson) as QuotaReport;
    expect(quota.readings).toHaveLength(3);
  });

  it("formats tokens and money", () => {
    expect(shortTokens(812)).toBe("812");
    expect(shortTokens(340_000)).toBe("340K");
    expect(shortTokens(35_000)).toBe("35K");
    expect(shortTokens(400_000_000)).toBe("400M");
    expect(shortTokens(841_300_000)).toBe("841.3M");
    expect(shortTokens(1_200_000_000)).toBe("1.2B");
    expect(shortTokens(2_500_000_000)).toBe("2.5B");
    expect(money(null)).toBe("—");
    expect(money(3.456)).toBe("$3.46");
  });

  it("stacks accounts per day, scaled to the tallest day", () => {
    const { bars, max } = stackedBars(
      { 1: day([["2026-11-01", 100, 1], ["2026-11-02", 50, 0.5]]), 2: day([["2026-11-01", 100, null]]) },
      false,
      200,
      100,
    );
    expect(max).toBe(200);
    expect(bars.map((b) => b.day)).toEqual(["2026-11-01", "2026-11-02"]);
    expect(bars[0]?.segments.map((s) => s.height)).toEqual([50, 50]);
    expect(bars[1]?.segments[0]?.height).toBe(25);
    const cost = stackedBars({ 1: day([["2026-11-01", 100, 2]]), 2: day([["2026-11-01", 5, null]]) }, true, 10, 10);
    expect(cost.max).toBe(2);
  });

  it("joins readings 3x the median spacing apart, held between 15 minutes and 2 hours", () => {
    const hour = 3600;
    expect(gapSeconds([0, 60, 120])).toBe(15 * 60);
    expect(gapSeconds([0, 600, 1200, 1800])).toBe(30 * 60);
    expect(gapSeconds([0, 3600, 7200])).toBe(2 * hour);
  });

  it("colours accounts by number from the shared palette, never a reading colour", () => {
    expect(accountColor(1)).toBe("#A78BFA");
    expect(accountColor(2, true)).toBe("#059669");
    expect(accountColor(9)).toBe("#A78BFA");
    expect(accountFill(1)).toBe("light-dark(#7C3AED, #A78BFA)");
    const reserved = ["#22D3EE", "#0891B2", "#FF9F0A", "#FACC15", "#A3E635", "#D64545"];
    for (const list of [ACCOUNT_PALETTE, ACCOUNT_PALETTE_LIGHT]) {
      expect(list.filter((c) => reserved.includes(c.toUpperCase()))).toEqual([]);
      expect(new Set(list).size).toBe(list.length);
    }
  });
});
