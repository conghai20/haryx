<script lang="ts">
  // The homepage's tide line (site Hero.astro), full width. It draws itself in once, from the left;
  // with reduced motion it is simply there.
  let { class: className = "h-20" }: { class?: string } = $props();
</script>

<svg class="pointer-events-none block w-full overflow-visible {className}" viewBox="0 0 1440 80" preserveAspectRatio="none" aria-hidden="true">
  <path d="M0 50 C 180 50, 300 18, 520 20 S 860 70, 1080 62 S 1340 30, 1440 34" class="tide-path fill-none stroke-live" stroke-width="1.25" vector-effect="non-scaling-stroke" pathLength="1" />
</svg>

<style>
  .tide-path { stroke-dasharray: 1; stroke-dashoffset: 0; }
  @media (prefers-reduced-motion: no-preference) {
    .tide-path { animation: draw 1.8s cubic-bezier(0.16, 1, 0.3, 1) 0.2s both; }
  }
  @keyframes draw { from { stroke-dashoffset: 1; } }
</style>
