<script lang="ts">
  import type { Snippet } from "svelte";
  let { title, blurb, children }: { title: string; blurb: string; children?: Snippet } = $props();
</script>

<header><h1>{title}</h1><p class="blurb">{blurb}</p></header>
{@render children?.()}

<style>
  header { margin-bottom: 1rem; }
  .blurb { color: var(--label-2); }
</style>
