// Pure scheduling so it can be tested without timers or Keycloak.
export const REFRESH_SKEW_S = 30;
export const MIN_DELAY_MS = 5_000;

/** Milliseconds until the token should be refreshed: `skew` seconds before `exp` (epoch seconds). */
export function msUntilRefresh(expSec: number, nowMs: number, skewS = REFRESH_SKEW_S): number {
  return Math.max(MIN_DELAY_MS, expSec * 1000 - skewS * 1000 - nowMs);
}

/** Keeps `refresh` running before every expiry. `refresh` returns the new exp, or null to stop. */
export function startRefresh(
  initialExp: number,
  refresh: () => Promise<number | null>,
  now: () => number = Date.now,
): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;
  const arm = (exp: number) => {
    timer = setTimeout(async () => {
      const next = await refresh();
      if (!stopped && next !== null) arm(next);
    }, msUntilRefresh(exp, now()));
  };
  arm(initialExp);
  return () => {
    stopped = true;
    clearTimeout(timer);
  };
}
