// What Accounts, Usage and Profiles read from the unlocked rows, opened and checked as the CLI
// checks them. Only the kinds those sections show are opened: `credential` rows (tokens) are never
// decrypted here, and the `settings` row is not needed.
import type { MascotStyle, UsageSync, UsageSyncDay } from "./types";
import { openDocument, rowKey, type SyncDocument, type SyncKey, type SyncRow } from "./rowcrypto";

/** The kinds this module opens. Anything else stays sealed. */
const OPENED_KINDS = new Set(["account", "app-prefs", "profiles", "usage"]);
/** `sync::usage::SCHEMA`: the usage body version this web reads. */
export const USAGE_SCHEMA = 1;

/** One synced Claude account (Rust `LocalAccount::values`): identity plus the user's settings. */
export interface SyncedAccount {
  email: string;
  organizationUuid: string;
  organizationName: string;
  alias: string | null;
  priority: number | null;
  disabled: boolean;
  threshold: number | null;
}

/** One profile the encrypted `profiles` index names, by the object key its file is stored under. */
export interface ProfileEntry {
  objectKey: string;
  name: string;
  bytes: number;
  /** Unix seconds. */
  pushedAt: number;
}

export interface Synced {
  accounts: SyncedAccount[];
  /** `app-prefs` `mascotStyles`, by `email|organizationUuid`. */
  mascotStyles: Record<string, MascotStyle>;
  /** Every machine's usage bodies that sit under their own key, by month then machine. */
  usage: UsageSync[];
  profiles: ProfileEntry[];
  /** Rows that did not open with this key or named another row; shown as a count, never trusted. */
  skipped: number;
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const text = (v: unknown) => (typeof v === "string" ? v : "");
const finite = (v: unknown) => (typeof v === "number" && Number.isFinite(v) ? v : null);
/** serde's u64 / i64: an integer (non-negative for u64). */
const isU64 = (v: unknown): v is number => Number.isSafeInteger(v) && (v as number) >= 0;
const isPct = (v: unknown) => v === null || v === undefined || finite(v) !== null;

/** The row key a usage body must sit under (Rust `usage::label`). */
export const usageLabel = (machineId: string, month: string) => `usage:${machineId}:${month}`;

/** An account document's values, or null when it names no email. */
export function parseAccount(values: Record<string, unknown>): SyncedAccount | null {
  const email = text(values.email).trim();
  if (!email) return null;
  const alias = text(values.alias).trim();
  return {
    email,
    organizationUuid: text(values.organizationUuid),
    organizationName: text(values.organizationName),
    alias: alias || null,
    priority: finite(values.priority),
    disabled: values.disabled === true,
    threshold: finite(values.threshold),
  };
}

/** `mascotStyles` from the `app-prefs` values: entries without a string species are dropped. */
export function parseMascotStyles(values: Record<string, unknown>): Record<string, MascotStyle> {
  const raw = values.mascotStyles;
  if (!isObject(raw)) return {};
  const out: Record<string, MascotStyle> = {};
  for (const [id, style] of Object.entries(raw)) {
    if (!isObject(style) || typeof style.species !== "string") continue;
    const hue = finite(style.hue);
    out[id] = { species: style.species, hue: hue !== null && hue >= 0 && hue <= 1 ? hue : null };
  }
  return out;
}

function parseDay(v: unknown): UsageSyncDay | null {
  if (!isObject(v) || typeof v.day !== "string" || typeof v.model !== "string") return null;
  if (!(v.account === null || v.account === undefined || typeof v.account === "string")) return null;
  const counts = [v.replies, v.input, v.output, v.cacheRead, v.cacheWrite5m, v.cacheWrite1h];
  if (!counts.every(isU64)) return null;
  return {
    day: v.day,
    account: (v.account as string | null | undefined) ?? null,
    model: v.model,
    replies: v.replies as number,
    input: v.input as number,
    output: v.output as number,
    cacheRead: v.cacheRead as number,
    cacheWrite5m: v.cacheWrite5m as number,
    cacheWrite1h: v.cacheWrite1h as number,
  };
}

/**
 * A usage body in the shape this version reads (Rust `usage::parse`: the whole body deserialises
 * as `UsageSync` and its schema is 1), else null. A usage-share row, in the same kind, is null here.
 */
export function parseUsage(values: Record<string, unknown>): UsageSync | null {
  const { schema, machineId, month, updatedAt, days, quota } = values;
  if (schema !== USAGE_SCHEMA || typeof machineId !== "string" || typeof month !== "string") return null;
  if (typeof updatedAt !== "string" || !Array.isArray(days) || !Array.isArray(quota)) return null;
  const parsedDays = days.map(parseDay);
  if (parsedDays.some((d) => d === null)) return null;
  const parsedQuota: UsageSync["quota"] = [];
  for (const q of quota) {
    if (!isObject(q) || !Number.isSafeInteger(q.at)) return null;
    if (!(q.account === null || q.account === undefined || typeof q.account === "string")) return null;
    if (!isPct(q.fiveHourPct) || !isPct(q.sevenDayPct)) return null;
    parsedQuota.push({
      at: q.at as number,
      account: (q.account as string | null | undefined) ?? null,
      fiveHourPct: finite(q.fiveHourPct),
      sevenDayPct: finite(q.sevenDayPct),
    });
  }
  return { schema, machineId, month, updatedAt, days: parsedDays as UsageSyncDay[], quota: parsedQuota };
}

/** The `profiles` index values: object key → `{name, bytes, pushedAt}` (Rust `profiles::entries`). */
export function parseProfileIndex(values: Record<string, unknown>): ProfileEntry[] {
  const out: ProfileEntry[] = [];
  for (const [objectKey, v] of Object.entries(values)) {
    if (!isObject(v) || typeof v.name !== "string" || !isU64(v.bytes) || !Number.isSafeInteger(v.pushedAt)) continue;
    out.push({ objectKey, name: v.name, bytes: v.bytes, pushedAt: v.pushedAt as number });
  }
  return out.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
}

/** Account order: by priority (unset last), then email. */
function byPriority(a: SyncedAccount, b: SyncedAccount): number {
  const pa = a.priority ?? Number.POSITIVE_INFINITY;
  const pb = b.priority ?? Number.POSITIVE_INFINITY;
  if (pa !== pb) return pa - pb;
  return a.email.localeCompare(b.email) || a.organizationUuid.localeCompare(b.organizationUuid);
}

/**
 * Opens the rows the sections need and keeps only what the CLI would trust: a document that names
 * its own row, and a usage body whose key is the one derived from its own machine and month
 * (Rust `usage::read_all`), so the server cannot pass one machine's usage off as another's.
 */
export async function decodeSynced(key: SyncKey, rows: readonly SyncRow[]): Promise<Synced> {
  const wanted = rows.filter((r) => OPENED_KINDS.has(r.kind));
  const opened = await Promise.all(wanted.map(async (row) => ({ row, opened: await openDocument(key, row) })));
  const out: Synced = { accounts: [], mascotStyles: {}, usage: [], profiles: [], skipped: 0 };
  for (const { row, opened: o } of opened) {
    if (o.status !== "doc") {
      out.skipped++;
      continue;
    }
    const doc: SyncDocument = o.doc;
    if (row.kind === "account") {
      const account = parseAccount(doc.values);
      if (account) out.accounts.push(account);
    } else if (row.kind === "app-prefs" && row.key === "") {
      out.mascotStyles = parseMascotStyles(doc.values);
    } else if (row.kind === "profiles" && row.key === "") {
      out.profiles = parseProfileIndex(doc.values);
    } else if (row.kind === "usage") {
      const body = parseUsage(doc.values);
      if (!body) continue; // a usage-share row, or a newer schema
      if ((await rowKey(key, usageLabel(body.machineId, body.month))) === row.key) out.usage.push(body);
      else out.skipped++;
    }
  }
  out.accounts.sort(byPriority);
  out.usage.sort((a, b) => a.month.localeCompare(b.month) || a.machineId.localeCompare(b.machineId));
  return out;
}
