// Machine logic kept apart from the views so it can be tested without a DOM.
import { relativeTime } from "@win/lib/text";

export const NAME_MAX = 64;

/** The trimmed name, or the reason it can't be used (the API enforces 1–64 characters too). */
export function validateName(raw: string): { ok: true; name: string } | { ok: false; error: string } {
  const name = raw.trim();
  if (name.length === 0) return { ok: false, error: "Enter a name." };
  if ([...name].length > NAME_MAX) return { ok: false, error: `Keep it to ${NAME_MAX} characters.` };
  return { ok: true, name };
}

export const lastSeen = (iso: string, now = Date.now()) => relativeTime(Date.parse(iso), now);
export const exactTime = (iso: string) => new Date(iso).toLocaleString();

export type SignOutStep = "keycloak" | "haryx";
export type SignOutResult = { ok: true } | { ok: false; failedAt: SignOutStep };

/**
 * Two steps (plan A7). Step 1 ends the machine's WeCoAp session with the user's own token; only
 * when it worked does step 2 tell HaryX. `skipKeycloak` retries step 2 alone.
 */
export async function signOutMachine(
  deps: { endSession: () => Promise<void>; record: () => Promise<void> },
  opts: { skipKeycloak?: boolean } = {},
): Promise<SignOutResult> {
  if (!opts.skipKeycloak) {
    try {
      await deps.endSession();
    } catch {
      return { ok: false, failedAt: "keycloak" };
    }
  }
  try {
    await deps.record();
  } catch {
    return { ok: false, failedAt: "haryx" };
  }
  return { ok: true };
}

export const signOutMessage = (failedAt: SignOutStep) =>
  failedAt === "keycloak"
    ? "Could not sign this machine out of WeCoAp. Nothing changed. Try again."
    : "This machine is signed out of WeCoAp, but HaryX could not record it. Retry to finish.";
