import { describe, expect, it } from "vitest";
import { ago, breakable, fillOf, overview, splitEmail, weekPath, WEEK_S } from "./overview";
import type { Synced, SyncedAccount } from "./synced";
import type { UsageSync } from "./types";

const NOW = 1_800_000_000;

const account = (email: string, extra: Partial<SyncedAccount> = {}): SyncedAccount => ({
  email,
  organizationUuid: "org",
  organizationName: "Org",
  alias: null,
  priority: null,
  disabled: false,
  threshold: null,
  ...extra,
});

const body = (machineId: string, quota: UsageSync["quota"], days: UsageSync["days"] = []): UsageSync => ({
  schema: 1,
  machineId,
  month: "2027-01",
  updatedAt: "2027-01-15T00:00:00Z",
  days,
  quota,
});

const synced = (accounts: SyncedAccount[], usage: UsageSync[]): Synced => ({
  accounts,
  mascotStyles: {},
  usage,
  profiles: [],
  skipped: 0,
});

describe("overview", () => {
  it("takes each account's newest reading across machines and names the last used", () => {
    const data = synced(
      [account("a@x.dev", { alias: "Main" }), account("b@x.dev")],
      [
        body("m1", [
          { at: NOW - 600, account: "A@x.dev", fiveHourPct: 40, sevenDayPct: 20 },
          { at: NOW - 3_600, account: "b@x.dev", fiveHourPct: 90, sevenDayPct: 70 },
        ]),
        body("m2", [{ at: NOW - 60, account: "a@x.dev", fiveHourPct: 62, sevenDayPct: 48 }]),
      ],
    );

    const out = overview(data, NOW);

    expect(out.accounts.map((a) => a.title)).toEqual(["Main", "b@x.dev"]);
    expect(out.accounts[0]!.reading).toEqual({ at: NOW - 60, machineId: "m2", fiveHourPct: 62, sevenDayPct: 48 });
    expect(out.lastUsed?.title).toBe("Main");
  });

  it("keeps only the last seven days of weekly readings, oldest first", () => {
    const data = synced(
      [account("a@x.dev")],
      [
        body("m1", [
          { at: NOW - 100, account: "a@x.dev", fiveHourPct: 1, sevenDayPct: 30 },
          { at: NOW - WEEK_S - 1, account: "a@x.dev", fiveHourPct: 1, sevenDayPct: 99 },
          { at: NOW - 5_000, account: "a@x.dev", fiveHourPct: 1, sevenDayPct: null },
          { at: NOW - 9_000, account: "a@x.dev", fiveHourPct: 1, sevenDayPct: 10 },
        ]),
      ],
    );

    expect(overview(data, NOW).accounts[0]!.week).toEqual([
      { at: NOW - 9_000, pct: 10 },
      { at: NOW - 100, pct: 30 },
    ]);
  });

  it("lists the machines whose usage names an account and has no last used without readings", () => {
    const day = { day: "2027-01-14", account: "a@x.dev", model: "m", replies: 1, input: 1, output: 1, cacheRead: 0, cacheWrite5m: 0, cacheWrite1h: 0 };
    const data = synced([account("a@x.dev")], [body("m2", [], [day]), body("m1", [], [day])]);

    const out = overview(data, NOW);

    expect(out.accounts[0]!.machineIds).toEqual(["m1", "m2"]);
    expect(out.lastUsed).toBeNull();
  });
});

describe("fillOf", () => {
  it("follows the three-step fill rule", () => {
    expect(fillOf(null)).toBe("normal");
    expect(fillOf(84.9)).toBe("normal");
    expect(fillOf(85)).toBe("amber");
    expect(fillOf(100)).toBe("red");
  });
});

describe("weekPath", () => {
  it("needs two points and maps the week onto the box", () => {
    expect(weekPath([{ at: NOW, pct: 50 }], NOW, 100, 20)).toBeNull();
    const p = weekPath([{ at: NOW - WEEK_S, pct: 0 }, { at: NOW, pct: 100 }], NOW, 100, 20)!;
    expect(p.line).toBe("M0.0 20.0 L100.0 0.0");
    expect(p.area).toBe("M0.0 20.0 L100.0 0.0 L100.0 20 L0.0 20 Z");
  });
});

describe("ago", () => {
  it("says how long ago a reading was taken", () => {
    expect(ago(NOW - 10, NOW)).toBe("just now");
    expect(ago(NOW - 120, NOW)).toBe("2 min ago");
    expect(ago(NOW - 7_200, NOW)).toBe("2 hr ago");
    expect(ago(NOW - 3 * 86_400, NOW)).toBe("3 days ago");
  });
});

describe("splitEmail and breakable", () => {
  it("splits an address at its last @ and leaves aliases alone", () => {
    expect(splitEmail("dev.team@example.co.uk")).toEqual({ local: "dev.team", domain: "@example.co.uk" });
    expect(splitEmail("Main account")).toBeNull();
  });
  it("offers breaks before dots, dashes and @", () => {
    expect(breakable("a.b-c@d.e")).toEqual(["a", ".b", "-c", "@d", ".e"]);
  });
});
