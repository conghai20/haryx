<script lang="ts">
  import { onMount } from "svelte";
  import { store } from "./lib/store.svelte";
  import Flyout from "./views/Flyout.svelte";
  import Dashboard from "./views/Dashboard.svelte";

  let { view }: { view: "flyout" | "dashboard" } = $props();
  let failed = $state<string | null>(null);

  onMount(() => {
    store.init().catch((e) => (failed = String(e)));
  });

  // The Theme preference pins light or dark; without it the page follows Windows.
  $effect(() => {
    const theme = store.prefs?.theme ?? "system";
    if (theme === "system") delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = theme;
  });
</script>

{#if failed}
  <div class="banner error" role="alert">HaryX could not start: {failed}</div>
{:else if view === "flyout"}
  <Flyout />
{:else}
  <Dashboard />
{/if}
