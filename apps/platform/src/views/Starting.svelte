<script lang="ts">
  import Mark from "../components/brand/Mark.svelte";
</script>

<div class="grid min-h-dvh place-items-center" role="status" aria-label="Starting HaryX">
  <Mark class="starting h-16 w-auto" />
</div>

<style>
  @media (prefers-reduced-motion: no-preference) {
    :global(.starting) { animation: breathe 1.6s cubic-bezier(0.65, 0, 0.35, 1) infinite; }
  }
  @keyframes breathe { 50% { opacity: 0.45; } }
</style>
