<script lang="ts">
  // Account cards (active first, then switch order) and an inspector for the selected one.
  import { flushSync, onMount } from "svelte";
  import { actions } from "../lib/actions";
  import { haryxAccount } from "../lib/haryx.svelte";
  import {
    NO_FILTER,
    STATUS_TITLES,
    applyFilter,
    filterActive,
    groupFilterFor,
    type AccountFilter,
    type StatusFilter,
  } from "../lib/filter";
  import { COLLAPSED_GROUPS_KEY, groupNames, groupSections, groupsInUse } from "../lib/groups";
  import { exhausted, resting } from "../lib/handoff";
  import { store } from "../lib/store.svelte";
  import {
    FIVE_HOUR_S,
    HIGH_PCT,
    SEVEN_DAY_S,
    displayName,
    needsRelogin,
    paceLine,
    pctLabel,
    statusNote,
    tier,
  } from "../lib/tide";
  import type { Account } from "../lib/types";
  import AccountMenu from "../components/AccountMenu.svelte";
  import AccountCard from "../components/AccountCard.svelte";
  import ClaudeSetupBanner from "../components/ClaudeSetupBanner.svelte";
  import NewGroupDialog from "../components/NewGroupDialog.svelte";
  import RenameGroupDialog from "../components/RenameGroupDialog.svelte";
  import Mascot from "../components/Mascot.svelte";
  import TideStaff from "../components/TideStaff.svelte";
  import Inspector from "./AccountInspector.svelte";
  import Icon from "../components/Icon.svelte";
  import MenuButton from "../components/MenuButton.svelte";

  let selectedNumber = $state<number | null>(null);

  /**
   * Opens the details panel on `n`, or closes it with null. Through a view transition, so the
   * cards move and resize with the panel instead of jumping to their new places; instant under
   * Reduce Motion or where view transitions are missing.
   */
  function showDetails(n: number | null) {
    if (n === selectedNumber) return;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduce || typeof document.startViewTransition !== "function") {
      selectedNumber = n;
      return;
    }
    document.startViewTransition(() => flushSync(() => (selectedNumber = n)));
  }
  /** The right-click menu: which card, and where the pointer was. */
  let menu = $state<{ account: Account; x: number; y: number } | null>(null);
  /** The group being renamed. */
  let renaming = $state<string | null>(null);
  /** The New Group dialog, with these accounts ticked; null while it is closed. */
  let newGroup = $state<number[] | null>(null);

  /** Elements a click may land on without closing the details panel. */
  const KEEPS_SELECTION = ".card, aside, button, select, input, label, a, [role='menu'], dialog";

  function closeOnEmptyClick(e: MouseEvent) {
    if (e.target instanceof Element && e.target.closest(KEEPS_SELECTION)) return;
    showDetails(null);
  }

  function openMenu(e: MouseEvent, a: Account) {
    e.preventDefault();
    menu = { account: a, x: e.clientX, y: e.clientY };
  }

  // Sections only once some account has a group; otherwise the pane is one plain grid.
  let grouped = $derived(groupsInUse(store.accounts));
  let filter = $state<AccountFilter>(NO_FILTER);
  let shown = $derived(applyFilter(store.accounts, filter));
  let live = $derived(shown.find((a) => a.active) ?? null);
  let sections = $derived(groupSections(shown, true));
  let names = $derived(groupNames(store.accounts));
  let collapsed = $state<string[]>(readCollapsed());

  /** Collapsed sections live in this PC's storage only; unreadable storage just means none. */
  function readCollapsed(): string[] {
    try {
      const raw = JSON.parse(localStorage.getItem(COLLAPSED_GROUPS_KEY) ?? "[]");
      return Array.isArray(raw) ? raw.filter((k): k is string => typeof k === "string") : [];
    } catch {
      return [];
    }
  }

  function toggle(key: string) {
    collapsed = collapsed.includes(key) ? collapsed.filter((k) => k !== key) : [...collapsed, key];
    try {
      localStorage.setItem(COLLAPSED_GROUPS_KEY, JSON.stringify(collapsed));
    } catch {
      // Storage blocked: the section still toggles for this session.
    }
  }
  let selected = $derived(store.accounts.find((a) => a.number === selectedNumber) ?? null);

  onMount(() => {
    void haryxAccount.init();
  });

  // The flyout's Info: select the account it named.
  $effect(() => {
    if (store.revealSlot == null) return;
    showDetails(store.revealSlot);
    store.revealSlot = null;
  });
</script>

<!-- A click on empty space (not on a card, a control or the details panel) closes the details
     panel; keyboard users have its close button. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="layout" class:with-inspector={selected} onclick={closeOnEmptyClick}>
  <div class="cards">
    <ClaudeSetupBanner />
    {#if store.accounts.length > 0}
      <div class="filters" role="group" aria-label="Filter accounts">
        {#if grouped}
          <label>
            <span class="caption secondary">Group</span>
            <select bind:value={filter.group}>
              <option value="all">All groups</option>
              {#each names as name (name)}<option value={groupFilterFor(name)}>{name}</option>{/each}
              <option value="ungrouped">Ungrouped</option>
            </select>
          </label>
        {/if}
        <label>
          <span class="caption secondary">Status</span>
          <select bind:value={filter.status}>
            {#each Object.entries(STATUS_TITLES) as [value, title] (value)}
              <option value={value as StatusFilter}>{title}</option>
            {/each}
          </select>
        </label>
        {#if filterActive(filter)}
          <span class="caption secondary" role="status">{shown.length} of {store.accounts.length} accounts</span>
          <button class="mini" onclick={() => (filter = NO_FILTER)}>Clear</button>
        {/if}
      </div>
    {/if}
    {#if live}
      <AccountCard
        account={live}
        live
        selected={live.number === selectedNumber}
        transitionName="account-live"
        onselect={() => showDetails(live?.number ?? null)}
        oncontextmenu={(e) => live && openMenu(e, live)}
      />
    {/if}
    {#each sections as section, i (section.key)}
      {@const isCollapsed = grouped && collapsed.includes(section.key)}
      {#if grouped}
        <h2 class="group-head">
          <button
            class="plain"
            aria-expanded={!isCollapsed}
            aria-controls="group-{i}"
            onclick={() => toggle(section.key)}
          >
            <span class="chevron" class:open={!isCollapsed}><Icon name="chevronRight" size={12} /></span>
            {section.name ?? "Ungrouped"}
            <span class="caption tertiary count">{section.accounts.length}</span>
          </button>
          {#if section.name}
            {@const name = section.name}
            <MenuButton
              plain
              align="start"
              ariaLabel="{name} actions"
              title="Rename or delete {name}"
              sections={[
                {
                  heading: null,
                  items: [
                    { label: "Rename group…", onpick: () => (renaming = name) },
                    { label: "Delete group…", danger: true, onpick: () => actions.deleteGroup(name) },
                  ],
                },
              ]}
            >
              <Icon name="more" />
            </MenuButton>
          {/if}
        </h2>
      {/if}
      <div class="group-cards" id="group-{i}" hidden={isCollapsed}>
        {#each section.accounts as a (a.number)}
          <AccountCard
            account={a}
            selected={a.number === selectedNumber}
            transitionName="account-{i}-{a.number}"
            onselect={() => showDetails(a.number)}
            oncontextmenu={(e) => openMenu(e, a)}
          />
        {/each}
      </div>
    {:else}
      {#if live}
        <!-- Only the account in use matches: it is shown above. -->
      {:else if store.accounts.length > 0}
        <div class="empty">
          <p>No accounts match.</p>
          <div class="actions">
            <button onclick={() => (filter = NO_FILTER)}>Clear filters</button>
          </div>
        </div>
      {:else}
      <div class="empty">
        <p>No accounts yet.</p>
        <p class="secondary">Sign in to Claude Code, then add that login. Or sign in a new account here.</p>
        <div class="actions">
          <button class="primary" onclick={() => actions.addCurrent()}>Add current login</button>
          <button onclick={() => actions.signIn(null)}>Sign in an account</button>
        </div>
      </div>
      {/if}
    {/each}
  </div>

  {#if selected}
    <Inspector account={selected} onclose={() => showDetails(null)} />
  {/if}
</div>

{#if haryxAccount.signedIn && haryxAccount.otherMachines.length > 0}
  <section class="others" aria-labelledby="others-title">
    <h2 id="others-title">On your other machines</h2>
    <p class="caption tertiary">Sign in here as that account in the browser; its alias, priority and the rest arrive with the next sync. Its login stays on this PC.</p>
    <ul>
      {#each haryxAccount.otherMachines as m (m.email)}
        <li>
          <div class="who">
            <strong class="truncate">{m.alias || m.email}</strong>
            <span class="caption tertiary truncate">{m.alias ? m.email : ""}{m.alias && m.organizationName ? " · " : ""}{m.organizationName ?? ""}</span>
          </div>
          <button class="mini primary" aria-label="Sign in {m.email} here" onclick={() => haryxAccount.signInHere(m)}>Sign in here</button>
          <button class="mini plain" aria-label="Forget {m.email}" onclick={() => haryxAccount.forget(m)}>Forget</button>
        </li>
      {/each}
    </ul>
  </section>
{/if}

{#if store.accounts.length > 0}
  <div class="footer">
    <button onclick={() => actions.addCurrent()}>Add current login</button>
    <button onclick={() => actions.signIn(null)}>Sign in another account</button>
    <button title="Make a group and choose its accounts" onclick={() => (newGroup = [])}>New group…</button>
  </div>
{/if}

{#if menu}
  <AccountMenu
    account={menu.account}
    x={menu.x}
    y={menu.y}
    onclose={() => (menu = null)}
    onnewgroup={(a) => (newGroup = [a.number])}
  />
{/if}

{#if renaming}
  <RenameGroupDialog name={renaming} onclose={() => (renaming = null)} />
{/if}

{#if newGroup}
  <NewGroupDialog initial={newGroup} onclose={() => (newGroup = null)} />
{/if}

<style>
  /* Same length and ease-out as macOS's details animation (smooth, 0.32 s). */
  :global(::view-transition-group(*)) {
    animation-duration: 320ms;
    animation-timing-function: cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  /* The details panel slides with the cards (see showDetails). */
  .layout > :global(aside) {
    view-transition-name: account-details;
  }
  .layout {
    display: grid;
    grid-template-columns: 1fr;
    gap: 16px;
  }
  .layout.with-inspector {
    grid-template-columns: 1fr minmax(300px, 380px);
  }
  /* Fixed-width cards (as macOS CardFlow): the details panel changes how many fit in a row, never
     how wide a card is, so opening it moves cards to the next row instead of stretching them. */
  .cards {
    --card-width: 264px;
    display: grid;
    grid-template-columns: repeat(auto-fill, min(var(--card-width), 100%));
    gap: 16px;
    align-content: start;
  }
  .filters {
    grid-column: 1 / -1;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
  }
  .filters label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  /* A section is a full-width row of the cards grid, and its cards share that grid's columns. */
  .group-head,
  .group-cards {
    grid-column: 1 / -1;
  }
  .group-cards {
    display: grid;
    grid-template-columns: subgrid;
    gap: 16px;
  }
  .group-cards[hidden] {
    display: none;
  }
  .group-head {
    margin: 4px 0 -4px;
    font-size: 13px;
    font-weight: 600;
  }
  .group-head {
    position: relative;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .group-head button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    font: inherit;
  }
  .chevron {
    display: inline-block;
    transition: transform 120ms ease;
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .count {
    font-weight: 400;
  }
  @media (prefers-reduced-motion: reduce) {
    .chevron {
      transition: none;
    }
  }
  .who {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    font-size: 13px;
  }
  .actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 6px;
  }
  .empty {
    grid-column: 1 / -1;
    padding: 24px;
    border-radius: var(--radius-card);
    background: var(--fill);
  }
  .others {
    margin-top: 20px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: 640px;
  }
  .others h2 {
    font-size: 13px;
    font-weight: 600;
    margin: 0;
  }
  .others p {
    margin: 0;
  }
  .others ul {
    list-style: none;
    margin: 4px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .others li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: var(--radius-card);
    background: var(--fill);
  }
  .others .who {
    font-size: 12px;
  }
  .footer {
    display: flex;
    gap: 8px;
    margin-top: 16px;
  }
</style>
