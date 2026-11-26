import { describe, expect, it } from "vitest";
import { groupNames, groupOrdered, groupSections, groupsInUse, sharesGroup } from "./groups";
import type { Account } from "./types";

const acct = (number: number, group: string | null, active = false): Account => ({
  number,
  email: `a${number}@x.com`,
  active,
  usageStatus: "ok",
  group,
});

describe("account groups", () => {
  it("sections the active group first, then by name, then ungrouped", () => {
    const sections = groupSections([
      acct(2, "Home", true),
      acct(1, null),
      acct(3, "client"),
      acct(4, "home"),
      acct(5, "Beta"),
    ]);
    expect(sections.map((s) => s.name)).toEqual(["Home", "Beta", "client", null]);
    expect(sections[0]?.accounts.map((a) => a.number)).toEqual([2, 4]);
  });

  it("puts ungrouped first when the active account is ungrouped", () => {
    expect(groupSections([acct(1, null, true), acct(2, "Work")]).map((s) => s.name)).toEqual([null, "Work"]);
  });

  it("names each group once, ignoring case", () => {
    const accounts = [acct(1, "work"), acct(2, "Work"), acct(3, "Alpha"), acct(4, null)];
    expect(groupNames(accounts)).toEqual(["Alpha", "work"]);
    expect(groupsInUse(accounts)).toBe(true);
    expect(groupsInUse([acct(1, null)])).toBe(false);
    expect(sharesGroup(acct(1, "Work"), acct(2, "WORK"))).toBe(true);
  });

  it("clusters the others by group, the active account's first, and leaves a groupless list alone", () => {
    const others = [acct(2, null), acct(3, "Work"), acct(4, "Home"), acct(5, null), acct(6, "work")];
    expect(groupOrdered(others, acct(1, "Work", true)).map((a) => a.number)).toEqual([3, 6, 4, 2, 5]);
    expect(groupOrdered(others, null).map((a) => a.number)).toEqual([4, 3, 6, 2, 5]);
    const plain = [acct(3, null), acct(2, null)];
    expect(groupOrdered(plain, acct(1, null, true))).toBe(plain);
  });
});
