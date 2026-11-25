import { describe, expect, it } from "vitest";
import { FIVE_HOUR_S, canRelogin, expectedPct, ordered, paceLine, shortName, tier } from "./tide";
import type { Account } from "./types";

const NOW = Date.parse("2026-11-24T12:00:00Z");
const inHours = (h: number) => new Date(NOW + h * 3600_000).toISOString();

describe("pace", () => {
  it("prefers the CLI's expected pace", () => {
    expect(expectedPct({ pct: 10, expectedPct: 42 }, FIVE_HOUR_S, NOW)).toBe(42);
  });

  it("falls back to linear over the window from the reset time", () => {
    // 2h left of 5h: 60% of the window has passed.
    expect(expectedPct({ pct: 10, resetsAt: inHours(2) }, FIVE_HOUR_S, NOW)).toBeCloseTo(60);
  });

  it("names the delta only at 5 or more points", () => {
    expect(paceLine({ pct: 70, expectedPct: 60 }, NOW)).toEqual({ text: "10 pts ahead of pace", warn: true });
    expect(paceLine({ pct: 50, expectedPct: 60 }, NOW)).toEqual({ text: "10 pts under pace", warn: false });
    expect(paceLine({ pct: 62, expectedPct: 60 }, NOW)).toEqual({ text: "on pace", warn: false });
    expect(paceLine({ pct: 62 }, NOW)).toBeNull();
  });
});

describe("three-step fill", () => {
  it("is live on pace, warn ahead or at 85%, limit at 100%", () => {
    expect(tier({ pct: 40, expectedPct: 40 }, FIVE_HOUR_S, NOW)).toBe("live");
    expect(tier({ pct: 50, expectedPct: 40 }, FIVE_HOUR_S, NOW)).toBe("warn");
    expect(tier({ pct: 86 }, FIVE_HOUR_S, NOW)).toBe("warn");
    expect(tier({ pct: 100 }, FIVE_HOUR_S, NOW)).toBe("limit");
    expect(tier({ pct: 20, aheadOfPace: true }, FIVE_HOUR_S, NOW)).toBe("warn");
    expect(tier(null, FIVE_HOUR_S, NOW)).toBe("unknown");
  });
});

describe("accounts", () => {
  const acct = (n: number, active: boolean, alias?: string): Account => ({
    number: n,
    email: `u${n}@x.com`,
    active,
    usageStatus: "ok",
    alias,
  });

  it("puts the active account first and keeps switch order", () => {
    expect(ordered([acct(1, false), acct(2, true), acct(3, false)]).map((a) => a.number)).toEqual([2, 1, 3]);
  });

  it("short name is the alias or the email's local part", () => {
    expect(shortName(acct(1, false, "work"))).toBe("work");
    expect(shortName(acct(2, false))).toBe("u2");
  });

  it("token and API-key slots cannot sign in again", () => {
    expect(canRelogin(acct(1, false))).toBe(true);
    expect(canRelogin({ ...acct(2, false), usageStatus: "api_key" })).toBe(false);
    expect(canRelogin({ ...acct(3, false), email: "abc@token.local" })).toBe(false);
  });
});
