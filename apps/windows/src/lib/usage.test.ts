import { describe, expect, it } from "vitest";
import reportJson from "../../../../packages/contracts/haryx/usage-report.json?raw";
import quotaJson from "../../../../packages/contracts/haryx/usage-quota.json?raw";
import { money, quotaLine, shortTokens, stackedBars, type QuotaReport, type UsageReport } from "./usage";


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
    expect(shortTokens(340_000)).toBe("340.0k");
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

  it("draws one window of one account", () => {
    const readings = [
      { at: 0, account: 1, fiveHour: 0, sevenDay: 50 },
      { at: 100, account: 1, fiveHour: 105, sevenDay: null },
      { at: 50, account: 2, fiveHour: 10, sevenDay: 10 },
    ];
    expect(quotaLine(readings, 1, "fiveHour", 0, 100, 100, 105)).toBe("0.0,105.0 100.0,0.0");
    expect(quotaLine(readings, 1, "sevenDay", 0, 100, 100, 105)).toBe("0.0,55.0");
  });
});
