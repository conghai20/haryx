// The `keyring` row: `{"wrapped": base64(age scrypt file of the sync key), "check": row_key("keyring-check")}`.
// Unwrapping is SyncKey::unwrap in cli/crates/haryx-core/src/sync/crypto.rs, with the same refusals.
// scrypt at log_n 18 takes most of a second and is synchronous, so the app calls this from a worker.
import { Decrypter } from "age-encryption";
import { CHECK_LABEL, importSyncKey, rowKey } from "./rowcrypto";

/** The heaviest keyring the CLI opens (MAX_UNWRAP_LOG_N); every keyring is wrapped at 18. */
export const MAX_UNWRAP_LOG_N = 20;

export type KeyringErrorCode = "wrong_passphrase" | "excessive_work" | "unreadable";

export class KeyringError extends Error {
  constructor(
    readonly code: KeyringErrorCode,
    message: string,
  ) {
    super(message);
  }
}

const unreadable = () => new KeyringError("unreadable", "The keyring row is not one this HaryX reads.");
const wrongPassphrase = () => new KeyringError("wrong_passphrase", "That is not your sync passphrase.");

export function excessiveWork(logN: number): KeyringError {
  return new KeyringError(
    "excessive_work",
    `Your sync keyring was locked with a scrypt work factor of ${logN}, more than HaryX opens (up to ${MAX_UNWRAP_LOG_N}). ` +
      "On a machine where sync is still unlocked, update HaryX and run 'haryx sync passphrase' to lock it again.",
  );
}

function parse(body: string): { cipher: Uint8Array; check: string } {
  try {
    const ring = JSON.parse(body) as { wrapped?: unknown; check?: unknown };
    if (typeof ring.wrapped !== "string" || typeof ring.check !== "string") throw unreadable();
    return { cipher: Uint8Array.from(atob(ring.wrapped.trim()), (c) => c.charCodeAt(0)), check: ring.check };
  } catch {
    throw unreadable();
  }
}

/** The `log_n` of the scrypt stanza in an age header, or null when there is none. */
export function scryptLogN(cipher: Uint8Array): number | null {
  // The header is ASCII and ends at "---"; 512 bytes is far past any scrypt header.
  const head = new TextDecoder("latin1").decode(cipher.subarray(0, 512));
  const line = head.split("\n").find((l) => l.startsWith("-> scrypt "));
  const logN = line ? Number(line.split(" ")[3]) : NaN;
  return Number.isInteger(logN) ? logN : null;
}

/**
 * The sync key's `AGE-SECRET-KEY-1…` text from a keyring row. A keyring above log_n 20 is refused
 * before any scrypt work; a wrong passphrase is `wrong_passphrase`; a key whose check does not
 * match the row's is `unreadable`, as in the CLI.
 */
export async function unwrapKeyring(body: string, passphrase: string): Promise<string> {
  const { cipher, check } = parse(body);
  const logN = scryptLogN(cipher);
  if (logN === null) throw unreadable();
  if (logN > MAX_UNWRAP_LOG_N) throw excessiveWork(logN);
  const decrypter = new Decrypter();
  decrypter.addPassphrase(passphrase);
  let secret: string;
  try {
    secret = await decrypter.decrypt(cipher, "text");
  } catch {
    throw wrongPassphrase();
  }
  let key;
  try {
    key = await importSyncKey(secret);
  } catch {
    throw unreadable();
  }
  if ((await rowKey(key, CHECK_LABEL)) !== check) throw unreadable();
  return secret.trim();
}
