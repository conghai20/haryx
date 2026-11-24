<script lang="ts">
  // The dashboard window: a sidebar grouped by domain ("AI", then "HaryX") and the selected pane.
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { store } from "../lib/store.svelte";
  import Lockup from "../components/Lockup.svelte";
  import Accounts from "../panes/Accounts.svelte";
  import Sessions from "../panes/Sessions.svelte";
  import Usage from "../panes/Usage.svelte";
  import AutoSwitch from "../panes/AutoSwitch.svelte";
  import Wake from "../panes/Wake.svelte";
  import Profiles from "../panes/Profiles.svelte";
  import Tools from "../panes/Tools.svelte";
  import Settings from "../panes/Settings.svelte";
  import About from "../panes/About.svelte";

  const panes = [
    { id: "accounts", group: "AI", title: "Accounts", icon: "◧", component: Accounts },
    { id: "usage", group: "AI", title: "Usage", icon: "▤", component: Usage },
    { id: "sessions", group: "AI", title: "Sessions", icon: "▸", component: Sessions },
    { id: "auto", group: "AI", title: "Auto-switch", icon: "⇄", component: AutoSwitch },
    { id: "wake", group: "AI", title: "Wake", icon: "◷", component: Wake },
    { id: "profiles", group: "AI", title: "Setup profiles", icon: "❏", component: Profiles },
    { id: "tools", group: "AI", title: "Tools", icon: "⚒", component: Tools },
    { id: "settings", group: "HaryX", title: "Settings", icon: "⚙", component: Settings },
    { id: "about", group: "HaryX", title: "About & updates", icon: "ⓘ", component: About },
  ] as const;

  type PaneId = (typeof panes)[number]["id"];
  let selected = $state<PaneId>("accounts");
  let current = $derived(panes.find((p) => p.id === selected) ?? panes[0]);

  onMount(() => {
    const off = listen<string>("open-pane", (e) => {
      if (panes.some((p) => p.id === e.payload)) selected = e.payload as PaneId;
    });
    return () => {
      off.then((f) => f());
    };
  });
</script>

<div class="dashboard">
  <nav aria-label="Sections">
    <div class="brand"><Lockup size={20} /></div>
    {#each ["AI", "HaryX"] as group (group)}
      <p class="group caption tertiary">{group}</p>
      {#each panes.filter((p) => p.group === group) as pane (pane.id)}
        <button
          class="item"
          class:selected={pane.id === selected}
          aria-current={pane.id === selected ? "page" : undefined}
          onclick={() => (selected = pane.id)}
        >
          <span class="icon" aria-hidden="true">{pane.icon}</span>{pane.title}
          {#if pane.id === "sessions" && store.sessions.sessions.length > 0}
            <span class="count caption">{store.sessions.sessions.length}</span>
          {/if}
          {#if pane.id === "auto" && store.autoRunning}<span class="dot" title="Running"></span>{/if}
          {#if pane.id === "about" && (store.update?.available || store.update?.state.phase === "ready")}
            <span class="dot" title="Update available"></span>
          {/if}
        </button>
      {/each}
    {/each}
  </nav>

  <main>
    <header>
      <h1>{current.title}</h1>
      {#if store.busy}<span class="caption secondary">{store.busy}…</span>{/if}
    </header>
    {#if store.message}
      <div class="banner" class:error={store.message.error} role="status">{store.message.text}</div>
    {/if}
    {#if store.error && !store.list}
      <div class="banner error" role="alert">{store.error}</div>
    {/if}
    <div class="pane">
      <current.component />
    </div>
  </main>
</div>

<style>
  .dashboard {
    display: grid;
    grid-template-columns: 200px 1fr;
    height: 100%;
  }
  nav {
    padding: 16px 8px;
    border-right: 1px solid var(--separator);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .brand {
    padding: 0 8px 12px;
  }
  .group {
    margin: 12px 8px 4px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    text-align: left;
    background: transparent;
    padding: 6px 8px;
  }
  .item.selected {
    background: var(--fill-strong);
    font-weight: 600;
  }
  .icon {
    width: 16px;
    text-align: center;
    color: var(--label-2);
  }
  .count {
    margin-left: auto;
  }
  .item .dot {
    margin-left: auto;
  }
  main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 20px 20px 8px;
  }
  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
  }
  main > .banner {
    margin: 0 20px 8px;
  }
  .pane {
    flex: 1;
    overflow: auto;
    padding: 8px 20px 20px;
  }
</style>
