<script lang="ts">
  // The usage reading: a flat bar, fill by the Three-Step Fill Rule, a 1px tick at the expected pace that
  // overhangs the bar by 3px. `off` (an account held out of rotation) trades cyan for grey, since cyan
  // means live; amber and red stay, because they are why an Off account is still tracked.
  import { expectedPct, tier } from "../lib/tide";
  import type { UsageWindow } from "../lib/types";

  let {
    window: w,
    windowS,
    now,
    height = 6,
    label = "usage",
    off = false,
  }: { window: UsageWindow | null | undefined; windowS: number; now: number; height?: number; label?: string; off?: boolean } =
    $props();

  let pct = $derived(Math.max(0, Math.min(100, w?.pct ?? 0)));
  let pace = $derived(expectedPct(w, windowS, now));
  let fill = $derived(tier(w, windowS, now));
</script>

<div
  class="staff"
  style:height="{height}px"
  role="meter"
  aria-label={label}
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={w?.pct ?? undefined}
>
  <div class="fill {fill}" class:off style:width="{pct}%"></div>
  {#if pace != null}
    <div class="tick" style:left="{Math.max(0, Math.min(100, pace))}%"></div>
  {/if}
</div>

<style>
  .staff {
    position: relative;
    width: 100%;
    background: var(--track);
    border-radius: var(--radius-staff);
  }
  .fill {
    height: 100%;
    border-radius: var(--radius-staff);
  }
  .fill.live {
    background: var(--cyan);
  }
  .fill.live.off {
    background: var(--label-2);
  }
  .fill.warn {
    background: var(--amber);
  }
  .fill.limit {
    background: var(--red);
  }
  .fill.unknown {
    background: transparent;
  }
  .tick {
    position: absolute;
    top: -3px;
    bottom: -3px;
    width: 1px;
    background: var(--label);
  }
</style>
