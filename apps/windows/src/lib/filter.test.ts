import { describe, expect, it } from "vitest";
import { NO_FILTER, applyFilter, filterActive, groupFilterFor, prunedFilter } from "./filter";
import type { Account } from "./types";

const acct = (number: number, group: string | null, disabled = false): Account => ({
  number,
  email: `a${number}@x.com`,
  active: false,
  usageStatus: "ok",
  group,
  disabled,
});

describe("account filter", () => {
  it("filters by group ignoring case and keeps ungrouped apart", () => {
    const accounts = [acct(1, "Work"), acct(2, "work"), acct(3, null), acct(4, "Home")];
    expect(applyFilter(accounts, { ...NO_FILTER, group: groupFilterFor("WORK") }).map((a) => a.number)).toEqual([1, 2]);
    expect(applyFilter(accounts, { ...NO_FILTER, group: "ungrouped" }).map((a) => a.number)).toEqual([3]);
    expect(applyFilter(accounts, NO_FILTER)).toHaveLength(4);
    expect(filterActive(NO_FILTER)).toBe(false);
  });

  it("splits on and off, and combines with a group", () => {
    const accounts = [acct(1, "Work"), acct(2, "Work", true), acct(3, null, true)];
    expect(applyFilter(accounts, { ...NO_FILTER, status: "on" }).map((a) => a.number)).toEqual([1]);
    expect(applyFilter(accounts, { ...NO_FILTER, status: "off" }).map((a) => a.number)).toEqual([2, 3]);
    expect(applyFilter(accounts, { group: groupFilterFor("work"), status: "off" }).map((a) => a.number)).toEqual([2]);
    expect(filterActive({ ...NO_FILTER, status: "off" })).toBe(true);
  });

  it("falls back to all groups when the filtered group was emptied", () => {
    const accounts = [acct(1, "Home")];
    expect(prunedFilter({ group: groupFilterFor("work"), status: "on" }, accounts)).toEqual({ group: "all", status: "on" });
    expect(prunedFilter({ group: groupFilterFor("home"), status: "all" }, accounts).group).toBe("group:home");
  });

  it("matches a group among several", () => {
    const accounts: Account[] = [
      { number: 1, email: "a1@x.com", active: false, usageStatus: "ok", group: "Work", groups: ["Work", "Home"] },
      { number: 2, email: "a2@x.com", active: false, usageStatus: "ok", group: "Home", groups: ["Home"] },
    ];
    expect(applyFilter(accounts, { ...NO_FILTER, group: groupFilterFor("home") }).map((a) => a.number)).toEqual([1, 2]);
    expect(applyFilter(accounts, { ...NO_FILTER, group: groupFilterFor("work") }).map((a) => a.number)).toEqual([1]);
  });
});
