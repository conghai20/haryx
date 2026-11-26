// The Accounts pane's filter: by group and by rotation status. Kept per window, never synced.
// Mirrors macOS `AccountFilter`.
import { accountGroups, isInGroup } from "./groups";
import type { Account } from "./types";

/** "all", "ungrouped", or a group's case-insensitive key prefixed with "group:". */
export type GroupFilter = "all" | "ungrouped" | `group:${string}`;
export type StatusFilter = "all" | "on" | "off";

export interface AccountFilter {
  group: GroupFilter;
  status: StatusFilter;
}

export const NO_FILTER: AccountFilter = { group: "all", status: "all" };

export const STATUS_TITLES: Record<StatusFilter, string> = {
  all: "All",
  on: "On (in rotation)",
  off: "Off (held out)",
};

export function groupFilterFor(name: string): GroupFilter {
  return `group:${name.toLowerCase()}`;
}

export function filterActive(f: AccountFilter): boolean {
  return f.group !== "all" || f.status !== "all";
}

export function matchesFilter(a: Account, f: AccountFilter): boolean {
  if (f.group === "ungrouped" && accountGroups(a).length > 0) return false;
  if (f.group.startsWith("group:") && !isInGroup(a, f.group.slice("group:".length))) return false;
  if (f.status === "on") return !a.disabled;
  if (f.status === "off") return !!a.disabled;
  return true;
}

/**
 * The same filter with a group no account carries any more reset to all, so a filter never hides
 * everything because its group was emptied.
 */
export function prunedFilter(f: AccountFilter, accounts: Account[]): AccountFilter {
  if (!f.group.startsWith("group:")) return f;
  const key = f.group.slice("group:".length);
  return accounts.some((a) => isInGroup(a, key)) ? f : { ...f, group: "all" };
}

export function applyFilter(accounts: Account[], f: AccountFilter): Account[] {
  const effective = prunedFilter(f, accounts);
  return accounts.filter((a) => matchesFilter(a, effective));
}
