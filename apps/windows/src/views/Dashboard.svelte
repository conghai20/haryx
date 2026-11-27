<script lang="ts">
  // The dashboard window: a sidebar in sections (Monitor, Automate, Set up, then HaryX) and the
  // selected pane.
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { store } from "../lib/store.svelte";
  import { api } from "../lib/api";
  import { actions } from "../lib/actions";
  import { OPEN_PANE_EVENT, REVEAL_ACCOUNT_EVENT, type PaneId } from "../lib/nav";
  import Icon, { type IconName } from "../components/Icon.svelte";
  import MenuButton, { type MenuSection } from "../components/MenuButton.svelte";
  import Lockup from "../components/Lockup.svelte";
  import AccountFooter from "../components/AccountFooter.svelte";
  import Accounts from "../panes/Accounts.svelte";
  import Sessions from "../panes/Sessions.svelte";
  import Usage from "../panes/Usage.svelte";
  import AutoSwitch from "../panes/AutoSwitch.svelte";
  import Wake from "../panes/Wake.svelte";
  import Mappings from "../panes/Mappings.svelte";
  import Profiles from "../panes/Profiles.svelte";
  import Tools from "../panes/Tools.svelte";
  import AiSettings from "../panes/AiSettings.svelte";
  import Preferences from "../panes/Preferences.svelte";
  import Account from "../panes/Account.svelte";
  import About from "../panes/About.svelte";

  // Sidebar sections, in the macOS app's order and words. Monitor, Automate and Set up hold the AI
  // panes (the `haryx ai` commands); later domains become siblings of them. `wide` panes (cards,
  // charts, file lists) use the whole width; form panes are 720px, centred.
  const groups = ["Monitor", "Automate", "Set up", "HaryX"] as const;
  const panes = [
    { id: "accounts", group: "Monitor", title: "Accounts", icon: "accounts", wide: true, component: Accounts },
    { id: "usage", group: "Monitor", title: "Usage", icon: "usage", wide: true, component: Usage },
    { id: "sessions", group: "Monitor", title: "Sessions", icon: "sessions", wide: false, component: Sessions },
    { id: "auto", group: "Automate", title: "Auto-switch", icon: "autoSwitch", wide: true, component: AutoSwitch },
    { id: "wake", group: "Automate", title: "Wake", icon: "wake", wide: false, component: Wake },
    { id: "mappings", group: "Automate", title: "Folder mappings", icon: "mappings", wide: false, component: Mappings },
    { id: "profiles", group: "Set up", title: "Setup profiles", icon: "profiles", wide: false, component: Profiles },
    { id: "settings", group: "Set up", title: "AI settings", icon: "settings", wide: false, component: AiSettings },
    { id: "tools", group: "Set up", title: "Backup & maintenance", icon: "backup", wide: false, component: Tools },
    // Reached from the sidebar footer, not listed.
    { id: "account", group: "", title: "HaryX account", icon: "account", wide: false, component: Account },
    { id: "preferences", group: "HaryX", title: "App preferences", icon: "preferences", wide: false, component: Preferences },
    { id: "about", group: "HaryX", title: "About & updates", icon: "about", wide: false, component: About },
  ] as const satisfies readonly { id: PaneId; icon: IconName; [k: string]: unknown }[];

  /** The Accounts toolbar's Switch menu: the same three ways macOS offers. */
  const switchMenu: MenuSection[] = [
    {
      heading: null,
      items: [
        { label: "Next account", onpick: () => actions.next() },
        { label: "Most headroom", onpick: () => actions.next("best") },
        { label: "Next available", onpick: () => actions.next("next-available") },
      ],
    },
  ];

  async function refresh() {
    try {
      await api.refresh();
    } catch (e) {
      store.fail(e, "Couldn't refresh");
    }
  }

  let selected = $state<PaneId>("accounts");
  let current = $derived(panes.find((p) => p.id === selected) ?? panes[0]);

  function openPane(id: string | null) {
    if (panes.some((p) => p.id === id)) selected = id as PaneId;
  }

  function reveal(slot: number) {
    selected = "accounts";
    store.revealSlot = slot;
  }

  onMount(() => {
    // A window opened for a pane or an account starts there (it missed the events below).
    const params = new URLSearchParams(location.search);
    openPane(params.get("pane"));
    const slot = Number(params.get("account"));
    if (Number.isInteger(slot) && slot > 0) reveal(slot);
    const offs = [
      listen<string>("open-pane", (e) => openPane(e.payload)),
      listen<number>("reveal-account", (e) => reveal(e.payload)),
    ];
    const onLocal = (e: Event) => openPane((e as CustomEvent<string>).detail);
    const onReveal = (e: Event) => reveal((e as CustomEvent<number>).detail);
    window.addEventListener(OPEN_PANE_EVENT, onLocal);
    window.addEventListener(REVEAL_ACCOUNT_EVENT, onReveal);
    return () => {
      offs.forEach((off) => off.then((f) => f()));
      window.removeEventListener(OPEN_PANE_EVENT, onLocal);
      window.removeEventListener(REVEAL_ACCOUNT_EVENT, onReveal);
    };
  });
</script>

<div class="dashboard">
  <nav aria-label="Sections">
    <div class="brand"><Lockup size={20} /></div>
    {#each groups as group (group)}
      <p class="group caption tertiary" id="nav-{group}">{group}</p>
      {#each panes.filter((p) => p.group === group) as pane (pane.id)}
        <button
          class="item"
          class:selected={pane.id === selected}
          aria-current={pane.id === selected ? "page" : undefined}
          onclick={() => (selected = pane.id)}
        >
          <span class="icon"><Icon name={pane.icon} /></span>{pane.title}
          {#if pane.id === "sessions" && store.sessions.sessions.length > 0}
            <span class="count caption">{store.sessions.sessions.length}</span>
          {/if}
          {#if pane.id === "auto" && store.autoRunning}<span class="dot" title="Running" aria-label="Running"></span>{/if}
          {#if pane.id === "about" && (store.update?.available || store.update?.state.phase === "ready")}
            <!-- Not cyan: an update is news, not something live. -->
            <span class="dot notice" title="Update available" aria-label="Update available"></span>
          {/if}
        </button>
      {/each}
    {/each}
    <div class="nav-footer"><AccountFooter onopen={() => (selected = "account")} /></div>
  </nav>

  <main>
    <header>
      <h1>{current.title}</h1>
      {#if store.busy}<span class="caption secondary" role="status">{store.busy}…</span>{/if}
      {#if selected === "accounts" && store.accounts.length > 0}
        <div class="tools">
          <MenuButton sections={switchMenu} ariaLabel="Switch account" title="Switch to another account" disabled={!!store.busy}>
            <Icon name="switch" />Switch
          </MenuButton>
          <button aria-label="Refresh accounts and usage" title="Refresh" onclick={refresh}><Icon name="refresh" /></button>
        </div>
      {/if}
    </header>
    {#if store.message}
      <div
        class="banner"
        class:error={store.message.error}
        role={store.message.error ? "alert" : "status"}
        title={store.message.detail}
      >
        <span>{store.message.text}</span>
        {#if store.message.error}
          <button class="plain dismiss" aria-label="Dismiss" title="Dismiss" onclick={() => store.dismiss()}>
            <Icon name="close" size={12} />
          </button>
        {/if}
      </div>
    {/if}
    {#if store.error && !store.list}
      <div class="banner error" role="alert" title={store.error}>
        HaryX couldn't read your accounts. Check that the haryx command-line tool is installed, then refresh.
      </div>
    {/if}
    <div class="pane">
      <div class="frame" class:wide={current.wide}>
        <current.component />
      </div>
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
  .nav-footer {
    margin-top: auto;
    padding: 8px 0 0;
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
  /* Selection is the accent at 18%, never a ring (DESIGN.md). */
  .item.selected {
    background: var(--selected);
    font-weight: 600;
  }
  .icon {
    display: inline-flex;
    color: var(--label-2);
  }
  .item.selected .icon {
    color: var(--label);
  }
  .dot.notice {
    background: var(--amber);
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
  .tools {
    margin-left: auto;
    display: flex;
    gap: 6px;
    align-self: center;
  }
  .tools button,
  .tools :global(button) {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  main > .banner {
    margin: 0 20px 8px;
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }
  main > .banner > span {
    flex: 1;
  }
  .dismiss {
    padding: 2px 4px;
  }
  /* Form panes: the form at most 720px wide and centred in a full-width scroll area (same as
     macOS paneFrame). Data panes (cards, charts, file lists) use the whole width. */
  .frame {
    max-width: 720px;
    margin-inline: auto;
  }
  .frame.wide {
    max-width: none;
  }
  .pane {
    flex: 1;
    overflow: auto;
    padding: 8px 20px 20px;
  }
</style>
