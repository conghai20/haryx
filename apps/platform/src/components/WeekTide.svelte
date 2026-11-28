<script lang="ts">
  // The last seven days of an account's weekly readings (DESIGN.md, Week Tide): the filled curve in
  // the fill colour over a track. No pace diagonal: sync carries no week start or reset, so the web
  // makes no pace claim. Drawn at its rendered width so strokes never scale.
  import { fillOf, weekPath, type WeekPoint } from "../lib/overview";
  let { points, now, height = 22, label }: { points: WeekPoint[]; now: number; height?: number; label: string } = $props();
  let width = $state(0);
  const path = $derived(width > 0 ? weekPath(points, now, width, height) : null);
  const last = $derived(points.length ? points[points.length - 1]!.pct : null);
  const step = $derived(fillOf(last));
  const area = $derived(step === "red" ? "fill-red" : step === "amber" ? "fill-amber" : "fill-ink-2");
  const line = $derived(step === "red" ? "stroke-red" : step === "amber" ? "stroke-amber" : "stroke-ink-2");
</script>

<div class="w-full" bind:clientWidth={width}>
  <svg class="block w-full rounded-[1.5px] bg-track" {height} viewBox="0 0 {Math.max(width, 1)} {height}" role="img" aria-label={label}>
    {#if path}
      <path class={area} stroke="none" d={path.area} fill-opacity="0.16" />
      <path class={line} fill="none" d={path.line} stroke-width="1.25" stroke-linejoin="round" />
    {/if}
  </svg>
</div>
