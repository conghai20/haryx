<script lang="ts">
  // The usage reading (DESIGN.md, Tide Staff): a flat bar on a track, filled by the Three-Step Fill
  // Rule. The web has no pace from sync, so it draws no pace tick.
  import { clampPct, fillOf } from "../lib/overview";
  let { pct, size = "md", live = false, label }: { pct: number | null; size?: "sm" | "md" | "lg"; live?: boolean; label: string } = $props();
  const fill = $derived(fillOf(pct));
  const h = $derived(size === "lg" ? "h-1.5" : size === "md" ? "h-[5px]" : "h-[3px]");
</script>

<div class="relative w-full overflow-hidden rounded-[1.5px] bg-track {h}" role="meter" aria-label={label} aria-valuemin="0" aria-valuemax="100" aria-valuenow={pct === null ? undefined : Math.round(pct)}>
  {#if pct !== null}
    <div
      class="h-full rounded-[1.5px] {fill === 'red' ? 'bg-red' : fill === 'amber' ? 'bg-amber' : live ? 'bg-live' : 'bg-ink-2'}"
      style:width="{clampPct(pct)}%"
    ></div>
  {/if}
</div>
