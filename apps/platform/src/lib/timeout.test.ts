import { afterEach, describe, expect, it, vi } from "vitest";
import { withTimeout } from "./timeout";

afterEach(() => vi.useRealTimers());

describe("withTimeout", () => {
  it("passes through a promise that settles in time", async () => {
    await expect(withTimeout(Promise.resolve(true), 50)).resolves.toBe(true);
    await expect(withTimeout(Promise.reject(new Error("no")), 50)).rejects.toThrow("no");
  });

  it("rejects when the promise never settles", async () => {
    vi.useFakeTimers();
    const pending = withTimeout(new Promise<boolean>(() => {}), 15_000);
    vi.advanceTimersByTime(15_000);
    await expect(pending).rejects.toThrow("timed out");
  });
});
