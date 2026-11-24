<script lang="ts">
  // Wing H mark on its navy tile, then the HARYX wordmark.
  let { size = 20, wordmark = true }: { size?: number; wordmark?: boolean } = $props();
  const wing =
    "M-16 -19 L-9.1 -19 L-9.1 -11.7 L-3.8 -5.4 C3 -10.5 11.5 -12.8 16 -19 C14 -12.5 7 -6.2 -4.5 2.5 L-16 -11.7 Z";
</script>

<span class="lockup" style:gap="{size * 0.4}px">
  <svg width={size} height={size} viewBox="0 0 64 64" aria-hidden="true">
    <rect width="64" height="64" rx={64 * 0.24} fill="#101827" />
    <g transform="translate(32 32) scale(0.92)">
      <path d={wing} fill="#F1F5F9" transform="rotate(180) scale(1.05 1.2)" />
      <path d={wing} fill="#22D3EE" transform="scale(1.05 1.2)" />
    </g>
  </svg>
  {#if wordmark}
    <span class="word" style:font-size="{size * 0.72}px" style:letter-spacing="{size * 0.08}px">HARYX</span>
  {/if}
</span>

<style>
  .lockup {
    display: inline-flex;
    align-items: center;
  }
  .word {
    font-family: var(--font-display);
    font-weight: 700;
    text-transform: uppercase;
  }
</style>
