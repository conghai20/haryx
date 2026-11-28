// Account groups (`haryx group`): labels the CLI stores on each account; an account can be in
// several. Names match ignoring case. Two accounts are in the same group when they share any
// group, and ungrouped accounts form a group of their own, as the CLI's automatic switching treats
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

/** Every group the account is in: `groups`, or `group` alone from an older CLI. */
export function accountGroups(a: Account): string[] {
  if (a.groups && a.groups.length > 0) return a.groups;
  return a.group ? [a.group] : [];
}

/** The account's groups, lowercased. */
export function groupKeys(a: Account): string[] {
  return accountGroups(a).map((g) => g.toLowerCase());
}

/** Whether the account is in `name` (ignoring case). */
export function isInGroup(a: Account, name: string): boolean {
  return groupKeys(a).includes(name.toLowerCase());
}

/** Its groups for a header or a list: "Work, Home", or "Ungrouped". */
export function groupLabel(a: Account): string {
  const groups = accountGroups(a);
  return groups.length > 0 ? groups.join(", ") : "Ungrouped";
}

/** In the same group: they share any group, or both are ungrouped. */
export function sharesGroup(a: Account, b: Account): boolean {
  const mine = groupKeys(a);
  const theirs = groupKeys(b);
  if (mine.length === 0 && theirs.length === 0) return true;
  return mine.some((k) => theirs.includes(k));
}

/** Whether any account carries a group. With none, nothing is sectioned. */
export function groupsInUse(accounts: Account[]): boolean {
  return accounts.some((a) => accountGroups(a).length > 0);
}

/** The groups in use, sorted by name, each in the first spelling met. */
export function groupNames(accounts: Account[]): string[] {
  const seen = new Map<string, string>();
  for (const a of accounts) for (const g of accountGroups(a)) if (!seen.has(g.toLowerCase())) seen.set(g.toLowerCase(), g);
  return [...seen.values()].sort((l, r) => l.localeCompare(r, undefined, { sensitivity: "base" }));
}

/**
 * `ordered` split into sections: the active account's groups first (by name), then the other
 * groups by name, then the ungrouped accounts. An account in several groups shows in each of their
 * sections, and each section keeps the order it was given. `excludingActive` leaves the active
 * account out (it has its own card) while still ordering by its groups; a section left empty is
 * dropped.
 */
export function groupSections(ordered: Account[], excludingActive = false): GroupSection[] {
  const active = ordered.find((a) => a.active);
  const activeKeys = active ? groupKeys(active) : [];
  const activeUngrouped = !!active && activeKeys.length === 0;
  const spelling = new Map<string, string>();
  for (const a of ordered) for (const g of accountGroups(a)) if (!spelling.has(g.toLowerCase())) spelling.set(g.toLowerCase(), g);
  const keys = [...spelling.keys()];
  if (ordered.some((a) => accountGroups(a).length === 0)) keys.push("");
  const isActive = (k: string) => (k === "" ? activeUngrouped : activeKeys.includes(k));
  keys.sort((l, r) => {
    if (isActive(l) !== isActive(r)) return isActive(l) ? -1 : 1;
    if ((l === "") !== (r === "")) return l === "" ? 1 : -1;
    return l < r ? -1 : l > r ? 1 : 0;
  });
  return keys.flatMap((key) => {
    const members = ordered.filter((a) => (key === "" ? accountGroups(a).length === 0 : groupKeys(a).includes(key)));
    const accounts = excludingActive ? members.filter((a) => !a.active) : members;
    return accounts.length > 0 ? [{ key, name: key === "" ? null : (spelling.get(key) ?? null), accounts }] : [];
  });
}

/**
 * `others` clustered by group in section order: accounts sharing a group with the active one first
 * (the CLI's automatic picks try them first), then the other groups by name, then the ungrouped
 * accounts. An account in several groups is listed once, at its first section. A list with no
 * groups is returned as it is.
 */
export function groupOrdered(others: Account[], active: Account | null): Account[] {
  const all = active ? [active, ...others] : others;
  if (!groupsInUse(all)) return others;
  const seen = new Set<number>();
  return groupSections(all)
    .flatMap((s) => s.accounts)
    .filter((a) => a !== active && !seen.has(a.number) && (seen.add(a.number), true));
}

/**
 * The flyout list's sections, as macOS `AccountGroups.listSections`: `others` (in switch order)
 * under each group once, the active account's groups first, an account in several groups at its
 * first section only. A list with no groups is one unnamed section.
 */
export function listSections(others: Account[], active: Account | null): GroupSection[] {
  const all = active ? [active, ...others] : others;
  if (!groupsInUse(all)) return others.length > 0 ? [{ key: "", name: null, accounts: others }] : [];
  const seen = new Set<number>();
  return groupSections(all, true)
    .map((s) => ({ ...s, accounts: s.accounts.filter((a) => !seen.has(a.number) && (seen.add(a.number), true)) }))
    .filter((s) => s.accounts.length > 0);
}

/** The CLI's longest group name (`GROUP_MAX_CHARS`). */
export const GROUP_MAX_CHARS = 32;

/**
 * A typed name as the CLI would store it: trimmed, and in the existing group's spelling when it
 * matches one ignoring case. Null when empty or longer than `GROUP_MAX_CHARS`. Mirrors macOS
 * `AccountGroups.normalizedName`.
 */
export function normalizeGroupName(typed: string, accounts: Account[]): string | null {
  const name = typed.trim();
  if (name.length === 0 || [...name].length > GROUP_MAX_CHARS) return null;
  return groupNames(accounts).find((g) => g.toLowerCase() === name.toLowerCase()) ?? name;
}
