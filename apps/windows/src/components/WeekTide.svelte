<script lang="ts">
  // The Week Tide (DESIGN.md): the week so far as a filled curve over the staff's track, coloured by
  // the Three-Step Fill Rule, against a 1px diagonal that is the linear pace. Stretches with no readings
  // are an estimated straight line, dashed and lighter. Drawn at its rendered pixel size, so strokes and
  // dashes never scale with the window. Nothing animates. macOS: `WeekTideView`.
  import { SEVEN_DAY_S, tier, weekTide } from "../lib/tide";
  import type { UsageWindow } from "../lib/types";

  let {
    window: w,
    readings,
    now,
    height = 16,
    off = false,
  }: { window: UsageWindow | null | undefined; readings: [number, number][]; now: number; height?: number; off?: boolean } =
    $props();

  let width = $state(0);
  let fill = $derived(tier(w, SEVEN_DAY_S, now));
  let segments = $derived(w?.pct != null && w.resetsAt ? weekTide(readings, w.resetsAt, w.pct, now) : []);

  function line(points: { x: number; y: number }[]): string {
    return points.map((p) => `${(p.x * width).toFixed(1)},${((1 - p.y) * height).toFixed(1)}`).join(" ");
  }

  function area(points: { x: number; y: number }[]): string {
    const first = points[0]!;
    const last = points.at(-1)!;
    return `${line(points)} ${(last.x * width).toFixed(1)},${height} ${(first.x * width).toFixed(1)},${height}`;
  }
</script>

<div class="tide" style:height="{height}px" bind:clientWidth={width} aria-hidden="true">
  {#if width > 0}
    <svg {width} {height} class="{fill}" class:off>
      <line x1="0" y1={height} x2={width} y2="0" class="pace" />
      {#each segments as s, i (i)}
        <polygon points={area(s.points)} class="area" class:estimated={s.estimated} />
        <polyline points={line(s.points)} class="curve" stroke-dasharray={s.estimated ? "2 2" : undefined} />
      {/each}
    </svg>
  {/if}
</div>

<style>
  .tide {
    width: 100%;
    background: var(--track);
    border-radius: var(--radius-staff);
    overflow: hidden;
  }
  svg {
    display: block;
    --c: var(--cyan);
  }
  svg.live.off {
    --c: var(--label-2);
  }
  svg.warn {
    --c: var(--amber);
  }
  svg.limit {
    --c: var(--red);
  }
  svg.unknown {
    --c: transparent;
  }
  .pace {
    stroke: var(--label-2);
    stroke-width: 1;
  }
  .area {
    fill: var(--c);
    fill-opacity: 0.32;
    stroke: none;
  }
  .area.estimated {
    fill-opacity: 0.14;
  }
  .curve {
    fill: none;
    stroke: var(--c);
    stroke-width: 1.25;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
</style>
