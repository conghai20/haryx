// Account groups (`haryx group`): a label the CLI stores on each account. Names match ignoring
// case, and ungrouped accounts form a group of their own, as the CLI's automatic switching treats
// them. Mirrors macOS `AccountGroups`.
import type { Account } from "./types";

export interface GroupSection {
  /** Case-insensitive key; "" for the ungrouped accounts. */
  key: string;
  /** The group's name as stored; null for the ungrouped accounts. */
  name: string | null;
  accounts: Account[];
}

/** Where the Accounts pane keeps its collapsed sections. This PC only, never synced. */
export const COLLAPSED_GROUPS_KEY = "haryx.collapsedGroups";

export function groupKey(a: Account): string {
  return (a.group ?? "").toLowerCase();
}

export function sharesGroup(a: Account, b: Account): boolean {
  return groupKey(a) === groupKey(b);
}

/** Whether any account carries a group. With none, nothing is sectioned. */
export function groupsInUse(accounts: Account[]): boolean {
  return accounts.some((a) => !!a.group);
}

/** The groups in use, sorted by name, each in the first spelling met. */
export function groupNames(accounts: Account[]): string[] {
  const seen = new Map<string, string>();
  for (const a of accounts) if (a.group && !seen.has(groupKey(a))) seen.set(groupKey(a), a.group);
  return [...seen.values()].sort((l, r) => l.localeCompare(r, undefined, { sensitivity: "base" }));
}

/**
 * `ordered` split into sections: the active account's group first, then the other groups by name,
 * then the ungrouped accounts. Each section keeps the order it was given.
 */
export function groupSections(ordered: Account[]): GroupSection[] {
  const activeAccount = ordered.find((a) => a.active);
  const activeKey = activeAccount ? groupKey(activeAccount) : null;
  const keys = [...new Set(ordered.map(groupKey))];
  keys.sort((l, r) => {
    if ((l === activeKey) !== (r === activeKey)) return l === activeKey ? -1 : 1;
    if ((l === "") !== (r === "")) return l === "" ? 1 : -1;
    return l < r ? -1 : l > r ? 1 : 0;
  });
  return keys.map((key) => {
    const accounts = ordered.filter((a) => groupKey(a) === key);
    return { key, name: accounts[0]?.group ?? null, accounts };
  });
}

/**
 * `others` clustered by group in section order: the active account's group first (the CLI's
 * automatic picks try it first), then the other groups by name, then the ungrouped accounts. Each
 * group keeps its given order. A list with no groups is returned as it is.
 */
export function groupOrdered(others: Account[], active: Account | null): Account[] {
  const all = active ? [active, ...others] : others;
  if (!groupsInUse(all)) return others;
  return groupSections(all)
    .flatMap((s) => s.accounts)
    .filter((a) => a !== active);
}
