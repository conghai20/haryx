// Sync rows in the browser, byte for byte as the CLI writes them (cli/crates/haryx-core/src/sync/
// crypto.rs). A row body is base64 of an age file encrypted to the account's X25519 sync key. A row
// key is lowercase hex HMAC-SHA256(label) under the subkey SHA-256("haryx-sync-row-key-v1" ‖ secret),
// where secret is the key's `AGE-SECRET-KEY-1…` text. The key lives in memory only.
import { Decrypter } from "age-encryption";

const HMAC_CONTEXT = "haryx-sync-row-key-v1";
/** The row-key label the keyring check is derived from; no row is ever named by it. */
export const CHECK_LABEL = "keyring-check";
const utf8 = new TextEncoder();

/** One row of `GET /api/v1/sync/rows`. */
export interface SyncRow {
  kind: string;
  key: string;
  body: string;
  version: number;
  updatedAt: string;
}

/** The decrypted body of every row but the keyring and usage rows (Rust `sync::Document`). */
export interface SyncDocument {
  kind: string;
  key: string;
  version: number;
  values: Record<string, unknown>;
  changedAt: number;
}

/** An unlocked sync key. Never serialised, never stored. */
export interface SyncKey {
  readonly decrypter: Decrypter;
  readonly hmac: CryptoKey;
}

export async function importSyncKey(secret: string): Promise<SyncKey> {
  // Rust re-encodes the identity before hashing it, and its text form is upper case.
  const identity = secret.trim().toUpperCase();
  if (!identity.startsWith("AGE-SECRET-KEY-1")) throw new Error("not an age X25519 identity");
  const decrypter = new Decrypter();
  decrypter.addIdentity(identity);
  const context = utf8.encode(HMAC_CONTEXT);
  const text = utf8.encode(identity);
  const joined = new Uint8Array(context.length + text.length);
  joined.set(context);
  joined.set(text, context.length);
  const subkey = await crypto.subtle.digest("SHA-256", joined);
  const hmac = await crypto.subtle.importKey("raw", subkey, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return Object.freeze({ decrypter, hmac });
}

/** The opaque row key for `label` (for example `account:you@example.com|org`): lowercase hex. */
export async function rowKey(key: SyncKey, label: string): Promise<string> {
  const mac = new Uint8Array(await crypto.subtle.sign("HMAC", key.hmac, utf8.encode(label)));
  return Array.from(mac, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** What a row key is derived from for an account, credential or usage-share row (Rust `sources::label`). */
export const accountLabel = (prefix: string, email: string, org: string) => `${prefix}:${email.trim().toLowerCase()}|${org}`;

/** Raw age bytes opened. `null`: not encrypted to this key (or not age at all). */
export async function openBytes(key: SyncKey, cipher: Uint8Array): Promise<Uint8Array | null> {
  try {
    return await key.decrypter.decrypt(cipher);
  } catch {
    return null;
  }
}

function fromBase64(text: string): Uint8Array | null {
  try {
    return Uint8Array.from(atob(text.trim()), (c) => c.charCodeAt(0));
  } catch {
    return null;
  }
}

/** A row body opened. `null`: it was not written with this key. */
export async function openRow(key: SyncKey, body: string): Promise<Uint8Array | null> {
  const cipher = fromBase64(body);
  return cipher ? openBytes(key, cipher) : null;
}

export type Opened = { status: "doc"; doc: SyncDocument } | { status: "foreign" } | { status: "unreadable" };

/**
 * A document row opened and checked against the row it came from, as the CLI does: a document names
 * its own kind, key and version inside the encryption, so a row the server moved or replayed is
 * `foreign`. `unreadable`: sealed with this key but in a shape this version does not read.
 */
export async function openDocument(key: SyncKey, row: SyncRow): Promise<Opened> {
  const plain = await openRow(key, row.body);
  if (!plain) return { status: "foreign" };
  let doc: SyncDocument;
  try {
    doc = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(plain)) as SyncDocument;
  } catch {
    return { status: "unreadable" };
  }
  if (typeof doc !== "object" || doc === null || typeof doc.values !== "object") return { status: "unreadable" };
  if (doc.kind !== row.kind || doc.key !== row.key || doc.version !== row.version) return { status: "foreign" };
  return { status: "doc", doc };
}

/** Where profile `name`'s file is stored: its object key (Rust `profiles::object_name`, minus the user prefix). */
export const profileObjectKey = (key: SyncKey, name: string) => rowKey(key, `profile:${name}`);

/** Profile `name`'s stored file, opened. `null`: not this key's, or framed as another profile. */
export async function openProfile(key: SyncKey, name: string, cipher: Uint8Array): Promise<Uint8Array | null> {
  const plain = await openBytes(key, cipher);
  const header = utf8.encode(`haryx-profile:${name}\n`);
  if (!plain || plain.length < header.length || header.some((b, i) => plain[i] !== b)) return null;
  return plain.slice(header.length);
}
