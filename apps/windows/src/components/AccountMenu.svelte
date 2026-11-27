<script lang="ts">
  // A card's right-click menu, as macOS AccountContextMenu: switch, run Claude, the account's groups
  // (pick one, make a new one, or leave it), rotation, copy email. Shown at the pointer; closes on
  // Escape, a click elsewhere, or a pick. Arrow keys move between items.
  import { onMount } from "svelte";
  import { actions } from "../lib/actions";
  import { accountGroups, groupNames, isInGroup } from "../lib/groups";
  import { store } from "../lib/store.svelte";
  import { displayName, needsRelogin } from "../lib/tide";
  import type { Account } from "../lib/types";

  let {
    account: a,
    x,
    y,
    onclose,
    onnewgroup,
  }: { account: Account; x: number; y: number; onclose: () => void; onnewgroup: (a: Account) => void } = $props();

  /** Keeps the menu inside the window when opened near an edge. */
  const EDGE_PX = 8;

  let menu: HTMLDivElement;
  let left = $state(0);
  let top = $state(0);
  let names = $derived(groupNames(store.accounts));

  onMount(() => {
    const r = menu.getBoundingClientRect();
    left = Math.max(EDGE_PX, Math.min(x, window.innerWidth - r.width - EDGE_PX));
    top = Math.max(EDGE_PX, Math.min(y, window.innerHeight - r.height - EDGE_PX));
    items()[0]?.focus();
  });

  function items(): HTMLButtonElement[] {
    return [...menu.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") return onclose();
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const list = items();
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    const step = e.key === "ArrowDown" ? 1 : -1;
    list[(i + step + list.length) % list.length]?.focus();
  }

  function pick(run: () => unknown) {
    onclose();
    void run();
  }

  async function copyEmail() {
    try {
      await navigator.clipboard.writeText(a.email);
      store.flash("Email copied");
    } catch (e) {
      store.fail(e, "Couldn't copy the email");
    }
  }
</script>

<svelte:window onkeydown={onkeydown} onresize={onclose} />

<!-- The backdrop only catches the click that closes the menu. -->
<div class="backdrop" role="presentation" onclick={onclose} oncontextmenu={(e) => { e.preventDefault(); onclose(); }}></div>
<div bind:this={menu} class="menu" role="menu" aria-label="{displayName(a)} actions" style:left="{left}px" style:top="{top}px">
  {#if needsRelogin(a)}
    <button role="menuitem" onclick={() => pick(() => actions.signIn(a))}>Sign in again…</button>
  {:else if !a.active}
    <button role="menuitem" disabled={!!store.busy} onclick={() => pick(() => actions.use(a))}>Switch to {displayName(a)}</button>
  {/if}
  <button role="menuitem" onclick={() => pick(() => actions.runAs(a))}>Run Claude</button>
  <button role="menuitem" onclick={() => pick(() => actions.runInFolder(a))}>Run Claude in folder…</button>

  <div class="sep" role="separator"></div>
  <p class="caption tertiary head" aria-hidden="true">Groups</p>
  <!-- Each group toggles: an account can be in several. -->
  {#each names as name (name)}
    {@const member = isInGroup(a, name)}
    <button
      role="menuitemcheckbox"
      aria-checked={member}
      disabled={!!store.busy}
      onclick={() => pick(() => (member ? actions.removeFromGroup(a, name) : actions.addToGroup(a, name)))}
    >
      <span class="check" aria-hidden="true">{member ? "✓" : ""}</span>{name}
    </button>
  {/each}
  <button role="menuitem" onclick={() => pick(() => onnewgroup(a))}><span class="check" aria-hidden="true"></span>New group…</button>
  {#if accountGroups(a).length > 1}
    <button role="menuitem" disabled={!!store.busy} onclick={() => pick(() => actions.clearGroups(a))}>
      <span class="check" aria-hidden="true"></span>Remove from All Groups
    </button>
  {/if}

  <div class="sep" role="separator"></div>
  <button role="menuitem" disabled={!!store.busy} onclick={() => pick(() => actions.setDisabled(a, !a.disabled))}>
    {a.disabled ? "Return to rotation" : "Hold out of rotation"}
  </button>
  <button role="menuitem" onclick={() => pick(copyEmail)}>Copy email</button>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 20;
  }
  .menu {
    position: fixed;
    z-index: 21;
    min-width: 220px;
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    padding: 4px;
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-banner);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.18);
  }
  button {
    text-align: left;
    background: transparent;
  }
  button:hover:not(:disabled),
  button:focus-visible {
    background: var(--fill);
  }
  .check {
    display: inline-block;
    width: 16px;
  }
  .head {
    margin: 2px 8px;
  }
  .sep {
    height: 1px;
    margin: 3px 4px;
    background: var(--separator);
  }
</style>
