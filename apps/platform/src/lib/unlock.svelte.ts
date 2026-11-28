// The sync lock for Accounts, Usage and Profiles. Unlocking fetches the rows, unwraps the keyring
// with the passphrase in a worker, and keeps the key in this object only: never in localStorage,
// sessionStorage, IndexedDB or a cookie. It locks again after 15 minutes without input, and when
// the page is hidden for good (`pagehide`: the tab closes, or the page goes into the back cache).
import { ApiError } from "./api";
import { KeyringError } from "./keyring";
import { importSyncKey, type SyncKey, type SyncRow } from "./rowcrypto";
import type { UnwrapReply, UnwrapRequest } from "./unwrap.worker";

export const IDLE_LOCK_MS = 15 * 60 * 1000;
const INPUT_EVENTS = ["pointerdown", "pointermove", "keydown", "wheel", "touchstart", "scroll"];

export type LockStatus = "locked" | "unlocking" | "unlocked";
export type UnlockErrorCode = KeyringError["code"] | "no_keyring" | "consent_required" | "network" | "server";

export interface UnlockError {
  code: UnlockErrorCode;
  message: string;
}

export interface SyncLockDeps {
  fetchRows: () => Promise<SyncRow[]>;
  /** The key's `AGE-SECRET-KEY-1…` text from a keyring row; rejects with a KeyringError. */
  unwrap: (body: string, passphrase: string) => Promise<string>;
  /** Where input and page-lifecycle events arrive: `window` in the app. */
  events: EventTarget;
  isVisible: () => boolean;
}

function describe(err: unknown): UnlockError {
  if (err instanceof KeyringError) return { code: err.code, message: err.message };
  if (err instanceof ApiError && err.kind === "consent_required")
    return { code: "consent_required", message: "Allow Sync in the Privacy center to read your synced data." };
  if (err instanceof ApiError && err.kind === "network")
    return { code: "network", message: "Could not reach HaryX. Check your connection and try again." };
  return { code: "server", message: "Could not load your synced data. Try again." };
}

export class SyncLock {
  status = $state<LockStatus>("locked");
  error = $state<UnlockError | null>(null);
  /** The rows as fetched at unlock (ciphertext; open them with `key`). */
  rows = $state.raw<SyncRow[]>([]);
  #key: SyncKey | null = null;
  #lastInput = 0;
  #timer: ReturnType<typeof setTimeout> | undefined;
  /** Bumped by every lock, so an unlock still in flight cannot land after it. */
  #generation = 0;

  constructor(private readonly deps: SyncLockDeps) {}

  get key(): SyncKey | null {
    return this.#key;
  }

  async unlock(passphrase: string): Promise<void> {
    if (this.status !== "locked") return;
    const generation = ++this.#generation;
    this.status = "unlocking";
    this.error = null;
    try {
      const rows = await this.deps.fetchRows();
      const ring = rows.find((r) => r.kind === "keyring" && r.key === "");
      if (!ring) {
        this.#fail(generation, {
          code: "no_keyring",
          message: "Sync is not set up for this account yet. Turn it on in the HaryX app first.",
        });
        return;
      }
      const key = await importSyncKey(await this.deps.unwrap(ring.body, passphrase));
      if (generation !== this.#generation) return;
      this.#key = key;
      this.rows = rows;
      this.status = "unlocked";
      this.#watch();
    } catch (err) {
      this.#fail(generation, describe(err));
    }
  }

  lock = (): void => {
    this.#generation++;
    this.#key = null;
    this.rows = [];
    this.status = "locked";
    clearTimeout(this.#timer);
    for (const type of INPUT_EVENTS) this.deps.events.removeEventListener(type, this.#onInput, { capture: true });
    this.deps.events.removeEventListener("pagehide", this.lock);
    this.deps.events.removeEventListener("visibilitychange", this.#onVisibility);
  };

  #fail(generation: number, error: UnlockError) {
    if (generation !== this.#generation) return;
    this.status = "locked";
    this.error = error;
  }

  #watch() {
    this.#lastInput = Date.now();
    for (const type of INPUT_EVENTS)
      this.deps.events.addEventListener(type, this.#onInput, { capture: true, passive: true });
    this.deps.events.addEventListener("pagehide", this.lock);
    this.deps.events.addEventListener("visibilitychange", this.#onVisibility);
    this.#schedule(IDLE_LOCK_MS);
  }

  #onInput = () => {
    this.#lastInput = Date.now();
  };

  // A sleeping laptop or a throttled background tab can fire the timer late: check on return too.
  #onVisibility = () => {
    if (this.deps.isVisible()) this.#checkIdle();
  };

  #schedule(ms: number) {
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => this.#checkIdle(), ms);
  }

  #checkIdle() {
    if (this.status !== "unlocked") return;
    const left = this.#lastInput + IDLE_LOCK_MS - Date.now();
    if (left <= 0) this.lock();
    else this.#schedule(left);
  }
}

/** Unwraps in a dedicated worker that is terminated once it answers. */
export function unwrapInWorker(body: string, passphrase: string): Promise<string> {
  const worker = new Worker(new URL("./unwrap.worker.ts", import.meta.url), { type: "module" });
  return new Promise((resolve, reject) => {
    worker.onmessage = (event: MessageEvent<UnwrapReply>) => {
      worker.terminate();
      const reply = event.data;
      if (reply.ok) resolve(reply.secret);
      else reject(new KeyringError(reply.code, reply.message));
    };
    worker.onerror = () => {
      worker.terminate();
      reject(new KeyringError("unreadable", "The keyring could not be opened in this browser."));
    };
    worker.postMessage({ body, passphrase } satisfies UnwrapRequest);
  });
}
