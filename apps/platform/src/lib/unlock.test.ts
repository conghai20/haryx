import { afterEach, describe, expect, it, vi } from "vitest";
import fixture from "./fixtures/sync-fixture.json";
import { ApiError } from "./api";
import { KeyringError, unwrapKeyring } from "./keyring";
import type { SyncRow } from "./rowcrypto";
import { IDLE_LOCK_MS, SyncLock } from "./unlock.svelte";

const rows: SyncRow[] = [
  { kind: "keyring", key: "", body: fixture.keyring, version: 1, updatedAt: "2026-11-27T00:00:00Z" },
  ...fixture.rows.map((r) => ({ kind: r.kind, key: r.key, body: r.body, version: r.version, updatedAt: "2026-11-27T00:00:00Z" })),
];

function setup(over: { fetchRows?: () => Promise<SyncRow[]>; unwrap?: (b: string, p: string) => Promise<string> } = {}) {
  const events = new EventTarget();
  let visible = true;
  const lock = new SyncLock({
    fetchRows: over.fetchRows ?? (async () => rows),
    // Unwrapping cannot be faked into success: the real check is verified against the row.
    unwrap: over.unwrap ?? (async () => fixture.secret),
    events,
    isVisible: () => visible,
  });
  const fire = (type: string) => events.dispatchEvent(new Event(type));
  return { lock, fire, hide: (v: boolean) => (visible = !v) };
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("SyncLock", () => {
  it("unlocks with the real keyring and keeps nothing in browser storage", { timeout: 30_000 }, async () => {
    const touched: string[] = [];
    const trap = (name: string) =>
      new Proxy({}, { get: (_t, prop) => (touched.push(`${name}.${String(prop)}`), () => undefined) });
    vi.stubGlobal("localStorage", trap("localStorage"));
    vi.stubGlobal("sessionStorage", trap("sessionStorage"));
    vi.stubGlobal("indexedDB", trap("indexedDB"));
    const { lock } = setup({ unwrap: unwrapKeyring });

    await lock.unlock(fixture.passphrase);

    expect(lock.status).toBe("unlocked");
    expect(lock.key).not.toBeNull();
    expect(lock.rows).toHaveLength(rows.length);
    expect(touched).toEqual([]);
    lock.lock();
  });

  it("says why a wrong passphrase failed and stays locked", async () => {
    const { lock } = setup({
      unwrap: async () => {
        throw new KeyringError("wrong_passphrase", "That is not your sync passphrase.");
      },
    });
    await lock.unlock("nope");
    expect(lock.status).toBe("locked");
    expect(lock.key).toBeNull();
    expect(lock.error).toEqual({ code: "wrong_passphrase", message: "That is not your sync passphrase." });
  });

  it("names a missing keyring, a missing consent and a network failure", async () => {
    const noRing = setup({ fetchRows: async () => rows.slice(1) });
    await noRing.lock.unlock("x");
    expect(noRing.lock.error?.code).toBe("no_keyring");

    const consent = setup({ fetchRows: () => Promise.reject(new ApiError("consent_required", 403, "consent_required")) });
    await consent.lock.unlock("x");
    expect(consent.lock.error?.code).toBe("consent_required");

    const offline = setup({ fetchRows: () => Promise.reject(new ApiError("network", 0)) });
    await offline.lock.unlock("x");
    expect(offline.lock.error?.code).toBe("network");
  });

  it("locks after 15 minutes without input, and input pushes it back", async () => {
    vi.useFakeTimers();
    const { lock, fire } = setup();
    await lock.unlock("x");
    expect(lock.status).toBe("unlocked");

    vi.advanceTimersByTime(IDLE_LOCK_MS - 1000);
    fire("keydown");
    vi.advanceTimersByTime(IDLE_LOCK_MS - 1000);
    expect(lock.status).toBe("unlocked");

    vi.advanceTimersByTime(1000);
    expect(lock.status).toBe("locked");
    expect(lock.key).toBeNull();
    expect(lock.rows).toEqual([]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("locks when the page is hidden for good", async () => {
    const { lock, fire } = setup();
    await lock.unlock("x");
    fire("pagehide");
    expect(lock.status).toBe("locked");
  });

  it("locks on return when the idle deadline passed while the timer could not fire", async () => {
    vi.useFakeTimers();
    const { lock, fire, hide } = setup();
    await lock.unlock("x");
    hide(true);
    vi.setSystemTime(Date.now() + IDLE_LOCK_MS + 1);
    fire("visibilitychange");
    expect(lock.status).toBe("unlocked");
    hide(false);
    fire("visibilitychange");
    expect(lock.status).toBe("locked");
  });

  it("drops an unlock that lands after a lock", async () => {
    let release!: (s: string) => void;
    const { lock } = setup({ unwrap: () => new Promise((r) => (release = r)) });
    const pending = lock.unlock("x");
    await vi.waitFor(() => expect(release).toBeTypeOf("function"));
    lock.lock();
    release(fixture.secret);
    await pending;
    expect(lock.status).toBe("locked");
    expect(lock.key).toBeNull();
  });
});
