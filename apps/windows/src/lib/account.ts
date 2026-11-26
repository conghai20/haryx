// HaryX account and sync: reading the CLI's JSON and turning its codes into words. The app never
// holds the HaryX session or the sync key; the CLI does, and this only reads what it prints.

import type { CliFailure, Overwrite, RunResult } from "./types";

/** The plan's floor: the wrapped key sits on the server, so a short passphrase could be guessed. */
export const MIN_PASSPHRASE = 12;

export const PASSPHRASE_WARNING =
  "HaryX cannot recover this passphrase. Any Mac or PC that is still unlocked can set a new one.";

export const DELETE_CONFIRM =
  "Delete your HaryX account? This removes everything synced, on every machine. Your Claude accounts on this PC are not touched.";

/** A command's JSON whatever its exit code (failures print `{"ok": false, …}` and exit 1). */
export function parseCli<T extends object>(r: RunResult): T | CliFailure {
  try {
    const value: unknown = JSON.parse(r.stdout.trim());
    if (value && typeof value === "object" && !Array.isArray(value)) return value as T;
  } catch {
    // Not JSON: an older CLI, or a crash before it printed anything.
  }
  return { ok: false, code: "local", message: r.message || `haryx exited with code ${r.status}` };
}

export function isFailure(value: object): value is CliFailure {
  return "ok" in value && value.ok === false;
}

/** Why a new or changed passphrase cannot be used yet, or null when it can. */
export function passphraseProblem(first: string, second: string): string | null {
  if (first.length < MIN_PASSPHRASE) return `Use at least ${MIN_PASSPHRASE} characters.`;
  if (first !== second) return "The two passphrases do not match.";
  return null;
}

/** What a failed sync means to the user, and what the pane should show for it. */
export interface SyncProblem {
  text: string;
  /** Show the passphrase form. */
  unlock: boolean;
  /** The HaryX session is gone: show the sign-in. */
  signedOut: boolean;
}

export function syncProblem(f: CliFailure): SyncProblem {
  const problem = (text: string, unlock = false, signedOut = false) => ({ text, unlock, signedOut });
  switch (f.code) {
    case "not_signed_in":
      return problem("Sign in to sync.", false, true);
    case "signed_out":
      return problem("Signed out of HaryX. Sign in again to sync.", false, true);
    case "locked":
      return problem("Enter your sync passphrase to unlock this PC.", true);
    case "key_changed":
      return problem("The sync passphrase changed on another machine. Enter it to unlock this PC.", true);
    case "wrong_passphrase":
      return problem("Can't unlock sync. Check the passphrase and try again.", true);
    case "busy":
      return problem("Another sync is running. It will finish on its own.");
    case "network":
    case "server":
      return problem("Sync unavailable, will retry.");
    default:
      return problem(f.message || "Sync did not finish.");
  }
}

/** One overwritten value as a line: what was kept, and what was lost. */
export function overwriteLine(o: Overwrite): string {
  const kept = o.kept === "remote" ? "another machine's value" : "this PC's value";
  const lost = o.lost == null ? "unset" : typeof o.lost === "string" ? o.lost : JSON.stringify(o.lost);
  return `${o.label}: kept ${kept}, replaced ${lost}`;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** "Last synced …": relative while recent, else the date. */
export function syncedAgo(iso: string | null, now: number): string {
  if (!iso) return "Never synced";
  const at = Date.parse(iso);
  if (Number.isNaN(at)) return "Never synced";
  const ago = Math.max(0, now - at);
  if (ago < MINUTE) return "Synced just now";
  if (ago < HOUR) return `Synced ${Math.floor(ago / MINUTE)} min ago`;
  if (ago < DAY) return `Synced ${Math.floor(ago / HOUR)} h ago`;
  return `Synced ${new Date(at).toLocaleString()}`;
}

/** The ways to sign in to HaryX; the id is `haryx login --provider`. */
export const SIGN_IN_PROVIDERS = [
  { id: "github", label: "GitHub" },
  { id: "google", label: "Google" },
] as const;

export type SignInProvider = (typeof SIGN_IN_PROVIDERS)[number]["id"];

export function providerName(provider: string): string {
  return provider === "github" ? "GitHub" : provider === "google" ? "Google" : provider;
}
