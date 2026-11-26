import { describe, expect, it } from "vitest";
import {
  GROUP_MAX_CHARS,
  accountGroups,
  groupLabel,
  isInGroup,
  groupNames,
  groupOrdered,
  groupSections,
  groupsInUse,
  normalizeGroupName,
  sharesGroup,
} from "./groups";
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

  it("trims a typed name, bounds it, and joins an existing spelling", () => {
    const accounts = [acct(1, "Work"), acct(2, null)];
    expect(normalizeGroupName("  work ", accounts)).toBe("Work");
    expect(normalizeGroupName(" Home ", accounts)).toBe("Home");
    expect(normalizeGroupName("   ", accounts)).toBeNull();
    expect(normalizeGroupName("x".repeat(GROUP_MAX_CHARS), accounts)).not.toBeNull();
    expect(normalizeGroupName("x".repeat(GROUP_MAX_CHARS + 1), accounts)).toBeNull();
  });

  it("can leave the active account out and drop the sections that empties", () => {
    const sections = groupSections([acct(1, "Solo", true), acct(2, "Work"), acct(3, "work")], true);
    expect(sections.map((s) => s.name)).toEqual(["Work"]);
    expect(sections[0]?.accounts.map((a) => a.number)).toEqual([2, 3]);
  });

  const multi = (number: number, groups: string[], active = false): Account => ({
    number,
    email: `a${number}@x.com`,
    active,
    usageStatus: "ok",
    group: groups[0] ?? null,
    groups,
  });

  it("treats accounts sharing any group as the same group and lists each in every section", () => {
    const a = multi(1, ["Work", "Home"], true);
    const b = multi(2, ["home"]);
    const c = multi(3, ["Client"]);
    const d = multi(4, []);
    expect(sharesGroup(a, b)).toBe(true);
    expect(sharesGroup(a, c)).toBe(false);
    expect(sharesGroup(a, d)).toBe(false);
    expect(groupLabel(a)).toBe("Work, Home");
    expect(groupLabel(d)).toBe("Ungrouped");
    const sections = groupSections([a, b, c, d]);
    expect(sections.map((s) => s.name)).toEqual(["Home", "Work", "Client", null]);
    expect(sections[0]?.accounts.map((x) => x.number)).toEqual([1, 2]);
    expect(sections[1]?.accounts.map((x) => x.number)).toEqual([1]);
  });

  it("lists an account in several groups once in switch order", () => {
    const active = multi(1, ["Work"], true);
    const others = [multi(2, ["Home", "Work"]), multi(3, ["Home"]), multi(4, [])];
    expect(groupOrdered(others, active).map((x) => x.number)).toEqual([2, 3, 4]);
  });

  it("reads an older CLI's single group", () => {
    const old = acct(1, "Work");
    expect(accountGroups(old)).toEqual(["Work"]);
    expect(isInGroup(old, "work")).toBe(true);
  });
});
