import { afterEach, describe, expect, it, vi } from "vitest";
import { MIN_DELAY_MS, msUntilRefresh, startRefresh } from "./refresh";

describe("msUntilRefresh", () => {
  it("fires 30 s before expiry", () => expect(msUntilRefresh(1000, 900_000)).toBe(70_000));
  it("never fires sooner than the floor", () => expect(msUntilRefresh(1000, 2_000_000)).toBe(MIN_DELAY_MS));
});

describe("startRefresh", () => {
  afterEach(() => vi.useRealTimers());
  it("refreshes, re-arms with the new expiry, and stops", async () => {
    vi.useFakeTimers();
    const refresh = vi.fn().mockResolvedValueOnce(901).mockResolvedValue(null);
    const stop = startRefresh(1000, refresh, () => 900_000);
    await vi.advanceTimersByTimeAsync(70_000);
    expect(refresh).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(MIN_DELAY_MS);
    expect(refresh).toHaveBeenCalledTimes(2);
    stop();
    await vi.advanceTimersByTimeAsync(10 * MIN_DELAY_MS);
    expect(refresh).toHaveBeenCalledTimes(2);
  });
  it("stop cancels a pending refresh", async () => {
    vi.useFakeTimers();
    const refresh = vi.fn();
    startRefresh(1000, refresh, () => 900_000)();
    await vi.advanceTimersByTimeAsync(100_000);
    expect(refresh).not.toHaveBeenCalled();
  });
});
