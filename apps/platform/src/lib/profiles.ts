// Profiles on the web: `GET /api/v1/profiles` lists files by object key only; their names live in
// the encrypted `profiles` index row. A download is the file `haryx sync profile pull` would import,
// checked against the name it is stored under (Rust `profiles::pull`).
import { openProfile, profileObjectKey, type SyncKey } from "./rowcrypto";
import type { ProfileEntry } from "./synced";

/** One file of `GET /api/v1/profiles` (server `sync.Profile`). */
export interface ServerProfile {
  name: string;
  bytes: number;
  updatedAt: string;
}

export interface ProfileItem extends ProfileEntry {
  /** Bytes stored on the server (encrypted), or null when the index names a file that is missing. */
  storedBytes: number | null;
  updatedAt: string | null;
}

/** The file extension `haryx ai profile import` reads (Rust `profile::FILE_EXTENSION`). */
export const FILE_EXTENSION = "haryx-profile";

/**
 * The index entries whose object key is the one derived from their name (so the index cannot point
 * one name at another's file), with what the server stores. `unnamed`: files no entry names.
 */
export async function joinProfiles(
  key: SyncKey,
  index: readonly ProfileEntry[],
  server: readonly ServerProfile[],
): Promise<{ items: ProfileItem[]; unnamed: number }> {
  const stored = new Map(server.map((p) => [p.name, p]));
  const items: ProfileItem[] = [];
  for (const entry of index) {
    if ((await profileObjectKey(key, entry.name)) !== entry.objectKey) continue;
    const file = stored.get(entry.objectKey);
    items.push({ ...entry, storedBytes: file?.bytes ?? null, updatedAt: file?.updatedAt ?? null });
  }
  const named = new Set(items.map((i) => i.objectKey));
  return { items, unnamed: server.filter((p) => !named.has(p.name)).length };
}

/** A download name for profile `name`: path separators and control characters replaced. */
export function fileName(name: string): string {
  const safe = name.replace(/[\u0000-\u001f\u007f/\\:*?"<>|]/g, "_").replace(/^\.+/, "_").trim() || "profile";
  return `${safe}.${FILE_EXTENSION}`;
}

export class ProfileError extends Error {}

/** The stored file opened as profile `name`, or a ProfileError saying why it cannot be. */
export async function openDownloaded(key: SyncKey, name: string, cipher: Uint8Array): Promise<Uint8Array> {
  const plain = await openProfile(key, name, cipher);
  if (!plain) throw new ProfileError(`The server's copy of “${name}” does not open with your sync key. Push it again from a machine.`);
  return plain;
}

/** A byte count for people: 812 B, 35 KB, 1.2 MB. */
export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1).replace(/\.0$/, "")} MB`;
}
