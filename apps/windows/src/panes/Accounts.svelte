<script lang="ts">
  // Account cards (active first, then switch order) and an inspector for the selected one.
  import { onMount } from "svelte";
  import { actions } from "../lib/actions";
  import { haryxAccount } from "../lib/haryx.svelte";
  import { COLLAPSED_GROUPS_KEY, groupSections, groupsInUse } from "../lib/groups";
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
  import ClaudeSetupBanner from "../components/ClaudeSetupBanner.svelte";
  import Mascot from "../components/Mascot.svelte";
  import TideStaff from "../components/TideStaff.svelte";
  import Inspector from "./AccountInspector.svelte";

  let selectedNumber = $state<number | null>(null);

  // Sections only once some account has a group; otherwise the pane is one plain grid.
  let grouped = $derived(groupsInUse(store.accounts));
  let sections = $derived(groupSections(store.accounts));
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
    selectedNumber = store.revealSlot;
    store.revealSlot = null;
  });
</script>

<div class="layout" class:with-inspector={selected}>
  <div class="cards">
    <ClaudeSetupBanner />
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
            <span class="chevron" class:open={!isCollapsed} aria-hidden="true">›</span>
            {section.name ?? "Ungrouped"}
            <span class="caption tertiary count">{section.accounts.length}</span>
          </button>
        </h2>
      {/if}
      <div class="group-cards" id="group-{i}" hidden={isCollapsed}>
        {#each section.accounts as a (a.number)}
          {@const five = a.usage?.fiveHour}
          {@const week = a.usage?.sevenDay}
          {@const pace = paceLine(five, store.now)}
          {@const running = store.runningFor(a)}
          <article
            class="card"
            class:selected={a.number === selectedNumber}
            class:off={resting(a)}
            aria-label={displayName(a)}
          >
            <button class="hit" aria-label="Inspect {displayName(a)}" onclick={() => (selectedNumber = a.number)}></button>
            <div class="head">
              <Mascot account={a} size={72} />
              <div class="who">
                <strong class="truncate">{displayName(a)}</strong>
                <span class="caption tertiary truncate">#{a.number} · {a.email}</span>
                <div class="badges">
                  {#if a.active}<span class="badge live">Active</span>{/if}
                  {#if a.plan?.label}<span class="badge">{a.plan.label}</span>{/if}
                  {#if a.disabled}<span class="badge">Off</span>{:else if exhausted(a)}<span class="badge">Limit</span>{/if}
                </div>
              </div>
            </div>
            <div class="reading">
              <span
                class="display figure"
                class:warn={tier(five, FIVE_HOUR_S, store.now) === "warn"}
                class:limit={tier(five, FIVE_HOUR_S, store.now) === "limit"}>{pctLabel(five?.pct)}</span
              >
              {#if running > 0}
                <span class="caption running"><span class="dot"></span> {running === 1 ? "running" : `${running} running`}</span>
              {/if}
            </div>
            <TideStaff window={five} windowS={FIVE_HOUR_S} now={store.now} height={5} label="5-hour usage" />
            <div class="caption line">
              {#if pace && pace.text !== "on pace"}<span class:warn-text={pace.warn}>{pace.text}</span>{:else}&nbsp;{/if}
            </div>
            <div class="caption line">
              <span class:warn-text={(week?.pct ?? 0) >= HIGH_PCT}>Week {pctLabel(week?.pct)}</span>
              {#if week?.countdown}<span class="tertiary"> · resets in {week.countdown}</span>{/if}
            </div>
            <TideStaff window={week} windowS={SEVEN_DAY_S} now={store.now} height={3} label="Weekly usage" />
            <div class="caption note">{statusNote(a) ?? ""}&nbsp;</div>
            <div class="actions">
              {#if needsRelogin(a)}
                <button class="mini sign-in" onclick={() => actions.signIn(a)}>Sign In Again</button>
              {:else if !a.active}
                <button class="mini primary" disabled={!!store.busy} onclick={() => actions.use(a)}>Switch</button>
              {/if}
              <button class="mini" onclick={() => actions.runAs(a)}>Claude as this account</button>
            </div>
          </article>
        {/each}
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
    {/each}
  </div>

  {#if selected}
    <Inspector account={selected} onclose={() => (selectedNumber = null)} />
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
          <button class="mini primary" aria-label="Sign in {m.email} here" onclick={() => haryxAccount.signInHere(m)}>Sign In Here</button>
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
  </div>
{/if}

<style>
  .layout {
    display: grid;
    grid-template-columns: 1fr;
    gap: 16px;
  }
  .layout.with-inspector {
    grid-template-columns: 1fr minmax(300px, 380px);
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(250px, 1fr));
    gap: 16px;
    align-content: start;
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
  .card {
    position: relative;
    padding: 14px;
    border-radius: var(--radius-card);
    background: var(--fill);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .card.selected {
    background: var(--selected);
  }
  .card.off {
    opacity: 0.6;
  }
  .hit {
    position: absolute;
    inset: 0;
    background: transparent;
    border-radius: var(--radius-card);
    padding: 0;
  }
  .hit:hover {
    background: transparent;
  }
  .head,
  .reading,
  .actions {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .head {
    align-items: flex-start;
    gap: 12px;
  }
  .who {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    font-size: 13px;
  }
  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
  }
  .reading {
    justify-content: space-between;
    margin: 6px 0 4px;
  }
  .figure {
    font-size: 28px;
  }
  .figure.warn,
  .warn-text {
    color: var(--amber);
  }
  .figure.limit {
    color: var(--red);
  }
  .running .dot {
    margin-right: 3px;
  }
  .line {
    margin-top: 4px;
    min-height: 14px;
  }
  .note {
    color: var(--amber);
    margin-top: 6px;
    min-height: 14px;
  }
  .actions {
    margin-top: 6px;
    flex-wrap: wrap;
  }
  .sign-in {
    background: var(--amber);
    color: white;
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
