import { describe, expect, it } from "vitest";
import { FIVE_HOUR_S, SEVEN_DAY_S, canRelogin, middleEllipsis, rotationDetail, rotationHeadline, weekTide, cardWeeklyPace, expectedPct, offersSignIn, ordered, paceLine, plansVary, rowName, runOutLabel, shortName, tier, usageFetchedLabel, weekIsTighter, weeklyPace } from "./tide";
import type { Account, Rotation } from "./types";

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

  it("reads the pace over the window it is given", () => {
    // 3.5 of 7 days left: half the week has passed, so 70% is 20 points ahead.
    expect(paceLine({ pct: 70, resetsAt: inHours(84) }, NOW, SEVEN_DAY_S)).toEqual({ text: "20 pts ahead of pace", warn: true });
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

  it("row name is the local part unless another account shares it (macOS AccountNames.rowName)", () => {
    const a = { ...acct(1, false), email: "hieunguyen@gmail.com" };
    const b = { ...acct(2, false), email: "dep1.pj02@corp.example" };
    const c = { ...acct(3, false), email: "Dep1.PJ02@other.example" };
    const d = acct(4, false, "work");
    const all = [a, b, c, d];
    expect(rowName(a, all)).toBe("hieunguyen");
    expect(rowName(b, all)).toBe("dep1.pj02@corp.example");
    expect(rowName(c, all)).toBe("Dep1.PJ02@other.example");
    expect(rowName(d, all)).toBe("work");
  });

  it("the week is tighter only past the floor and above the 5-hour (macOS Usage.weekIsTighter)", () => {
    expect(weekIsTighter({ fiveHour: { pct: 26 }, sevenDay: { pct: 75 } })).toBe(true);
    expect(weekIsTighter({ fiveHour: null, sevenDay: { pct: 60 } })).toBe(true);
    expect(weekIsTighter({ fiveHour: { pct: 90 }, sevenDay: { pct: 75 } })).toBe(false);
    expect(weekIsTighter({ fiveHour: { pct: 0 }, sevenDay: { pct: 3 } })).toBe(false);
    expect(weekIsTighter(null)).toBe(false);
  });

  it("plan labels show only when the accounts' plans differ", () => {
    const team = (n: number) => ({ ...acct(n, false), plan: { label: "Team" } });
    expect(plansVary([team(1), team(2)])).toBe(false);
    expect(plansVary([team(1), { ...acct(3, false), plan: { label: "Pro" } }])).toBe(true);
  });

  it("token and API-key slots cannot sign in again", () => {
    expect(canRelogin(acct(1, false))).toBe(true);
    expect(canRelogin({ ...acct(2, false), usageStatus: "api_key" })).toBe(false);
    expect(canRelogin({ ...acct(3, false), email: "abc@token.local" })).toBe(false);
  });

  it("offers Sign In Again when a re-login is needed or the login expires within a week", () => {
    expect(offersSignIn(acct(1, false), NOW)).toBe(false);
    expect(offersSignIn({ ...acct(2, false), usageStatus: "relogin_required" }, NOW)).toBe(true);
    expect(offersSignIn({ ...acct(3, false), loginExpiresAt: inHours(24 * 6) }, NOW)).toBe(true);
    expect(offersSignIn({ ...acct(4, false), loginExpiresAt: inHours(24 * 8) }, NOW)).toBe(false);
    expect(offersSignIn({ ...acct(5, false), loginExpiresAt: "not a date" }, NOW)).toBe(false);
    expect(offersSignIn({ ...acct(6, false), usageStatus: "api_key", loginExpiresAt: inHours(1) }, NOW)).toBe(false);
  });
});

describe("weekly pace", () => {
  it("reads ahead by the cards' 5-point threshold, not the CLI's 15-point flag", () => {
    expect(weeklyPace({ pct: 32, expectedPct: 17.6, aheadOfPace: false }, NOW)).toEqual({
      text: "14 pts ahead (expected 18%)",
      warn: true,
    });
  });

  it("reads under pace and on pace", () => {
    expect(weeklyPace({ pct: 69, expectedPct: 82.6 }, NOW)).toEqual({ text: "14 pts under (expected 83%)", warn: false });
    expect(weeklyPace({ pct: 20, expectedPct: 18 }, NOW)).toEqual({ text: "On pace (expected 18%)", warn: false });
  });

  it("says nothing without the CLI's expected pace", () => {
    expect(weeklyPace({ pct: 20, resetsAt: inHours(100) }, NOW)).toBeNull();
    expect(weeklyPace(null, NOW)).toBeNull();
  });
});

describe("usageFetchedLabel", () => {
  const acct = (n: number, _active: boolean): Account => ({
    number: n,
    email: `u${n}@x.com`,
    active: false,
    usageStatus: "ok",
  });

  it("says how old the reading is", () => {
    expect(usageFetchedLabel({ ...acct(1, false), usageAgeSeconds: 20 })).toBe("Usage fetched just now");
    expect(usageFetchedLabel({ ...acct(1, false), usageAgeSeconds: 185 })).toBe("Usage fetched 3 min ago");
    expect(usageFetchedLabel({ ...acct(1, false), usageAgeSeconds: 7300 })).toBe("Usage fetched 2 h ago");
  });

  it("names another machine's reading", () => {
    const a = { ...acct(1, false), usageAgeSeconds: 120, usageFromOtherMachine: true };
    expect(usageFetchedLabel(a)).toBe("Usage fetched 2 min ago, on another machine");
  });

  it("is absent without a reading", () => {
    expect(usageFetchedLabel(acct(1, false))).toBeNull();
  });
});

describe("card weekly pace", () => {
  const label = () => "Sat 10:15";
  const out = inHours(40);

  it("names the run-out when ahead and the week will not last", () => {
    expect(cardWeeklyPace({ pct: 32, expectedPct: 17.6, projectedExhaustionAt: out, willLastToReset: false }, NOW, label)).toEqual({
      text: "14 pts ahead of pace · runs out Sat 10:15",
      warn: true,
    });
  });

  it("leaves the run-out off when the week lasts to its reset", () => {
    expect(cardWeeklyPace({ pct: 40, expectedPct: 30, projectedExhaustionAt: out, willLastToReset: true }, NOW, label)).toEqual({
      text: "10 pts ahead of pace",
      warn: true,
    });
  });

  it("reads under pace and on pace without a run-out", () => {
    expect(cardWeeklyPace({ pct: 69, expectedPct: 82.6, projectedExhaustionAt: out, willLastToReset: false }, NOW, label)).toEqual({
      text: "14 pts under pace",
      warn: false,
    });
    expect(cardWeeklyPace({ pct: 20, expectedPct: 18, projectedExhaustionAt: out, willLastToReset: false }, NOW, label)).toEqual({
      text: "on pace",
      warn: false,
    });
  });

  it("says nothing without the CLI's expected pace", () => {
    expect(cardWeeklyPace({ pct: 20, resetsAt: inHours(100) }, NOW, label)).toBeNull();
  });

  it("formats a run-out as weekday and 24-hour clock, and skips a bad date", () => {
    expect(runOutLabel("not a date")).toBeNull();
    expect(runOutLabel("2026-11-28T22:05:00")).toMatch(/22:05/);
  });
});

describe("week tide", () => {
  const reset = 7 * 86_400;
  const start = reset - SEVEN_DAY_S;
  const iso = (s: number) => new Date(s * 1000).toISOString();
  const atMs = (hours: number) => (start + hours * 3600) * 1000;
  const at = (hours: number) => start + hours * 3600;

  it("draws the average as one estimated line with no readings", () => {
    const segs = weekTide([], iso(reset), 40, atMs(84));
    expect(segs).toHaveLength(1);
    expect(segs[0]!.estimated).toBe(true);
    expect(segs[0]!.points[0]).toEqual({ x: 0, y: 0 });
    expect(segs[0]!.points.at(-1)).toEqual({ x: 0.5, y: 0.4 });
  });

  it("joins readings from the start of the week to the origin", () => {
    const readings: [number, number][] = [];
    for (let h = 0.5; h <= 10; h += 0.5) readings.push([at(h), h]);
    const segs = weekTide(readings, iso(reset), 10.2, atMs(10.2));
    expect(segs.map((s) => s.estimated)).toEqual([false]);
    expect(segs[0]!.points[0]).toEqual({ x: 0, y: 0 });
  });

  it("estimates a gap rather than drawing it as read", () => {
    const readings: [number, number][] = [];
    for (let h = 0.25; h <= 2; h += 0.25) readings.push([at(h), h]);
    for (let h = 20; h <= 22; h += 0.25) readings.push([at(h), h]);
    const segs = weekTide(readings, iso(reset), 22, atMs(22.1));
    expect(segs.map((s) => s.estimated)).toEqual([false, true, false]);
  });

  it("leads with an estimate when history started mid-week, and ignores last week", () => {
    const readings: [number, number][] = [[start - 3600, 90]];
    for (let h = 80; h <= 84; h += 0.25) readings.push([at(h), h / 2]);
    const segs = weekTide(readings, iso(reset), 42, atMs(84.1));
    expect(segs[0]!.estimated).toBe(true);
    expect(segs.slice(1).every((s) => !s.estimated)).toBe(true);
    expect(segs.flatMap((s) => s.points).every((p) => p.y <= 0.43)).toBe(true);
  });
});

describe("rotation verdict", () => {
  const base: Rotation = { accounts: [1, 2], unknown: [], headroomPct: 120, burnPctPerHour: 1, nextResetAt: "2026-11-28T05:00:00Z", runsDryAt: null, lasts: true };

  it("says the week lasts", () => {
    expect(rotationHeadline(base)).toBe("The week lasts");
    expect(rotationDetail(base)).toMatch(/^2 accounts auto-switch can use carry you past the next reset, /);
  });

  it("says how long before the reset it runs dry", () => {
    const dry: Rotation = { ...base, accounts: [1], lasts: false, runsDryAt: "2026-11-27T15:00:00Z" };
    expect(rotationHeadline(dry)).toMatch(/^Runs dry /);
    expect(rotationDetail(dry)).toMatch(/^14 h before the next reset, .*, across 1 account auto-switch can use\.$/);
  });
});

describe("middleEllipsis", () => {
  it("keeps both ends of a long name", () => {
    expect(middleEllipsis("short@x.com", 24)).toBe("short@x.com");
    const cut = middleEllipsis("Dep1pj.claude02@ml.vn-cubesystem.com", 24);
    expect(cut).toHaveLength(24);
    expect(cut.startsWith("Dep1pj.claude02")).toBe(true);
    expect(cut.endsWith(".com")).toBe(true);
  });
});
