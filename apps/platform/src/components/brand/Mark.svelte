<script lang="ts">
  // The HaryX mark ("Wing H") as two halves, drawn like site/src/components/Mark.astro: upper half
  // cyan, lower half in ink.
  let { class: className = "", label }: { class?: string; label?: string } = $props();
  const HALF = "M-16 -19 L-9.1 -19 L-9.1 -11.7 L-3.8 -5.4 C3 -10.5 11.5 -12.8 16 -19 C14 -12.5 7 -6.2 -4.5 2.5 L-16 -11.7 Z";
</script>

<svg class={className} viewBox="-22 -30 44 60" role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : "true"}>
  <g transform="scale(1.228)">
    <path class="fill-ink" d={HALF} transform="rotate(180) scale(1.05 1.2)" />
    <path class="fill-[#22d3ee]" d={HALF} transform="scale(1.05 1.2)" />
  </g>
</svg>
