<script lang="ts">
  // Account cards (active first, then switch order) and an inspector for the selected one.
  import { actions } from "../lib/actions";
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
  import TideStaff from "../components/TideStaff.svelte";
  import Inspector from "./AccountInspector.svelte";

  let selectedNumber = $state<number | null>(null);
  let selected = $derived(store.accounts.find((a) => a.number === selectedNumber) ?? null);
</script>

<div class="layout" class:with-inspector={selected}>
  <div class="cards">
    {#each store.accounts as a (a.number)}
      {@const five = a.usage?.fiveHour}
      {@const week = a.usage?.sevenDay}
      {@const pace = paceLine(five, store.now)}
      {@const running = store.runningFor(a)}
      <article
        class="card"
        class:selected={a.number === selectedNumber}
        class:off={a.disabled}
        aria-label={displayName(a)}
      >
        <button class="hit" aria-label="Inspect {displayName(a)}" onclick={() => (selectedNumber = a.number)}></button>
        <div class="head">
          <div class="who">
            <strong class="truncate">{displayName(a)}</strong>
            <span class="caption tertiary truncate">#{a.number} · {a.email}</span>
          </div>
          <div class="badges">
            {#if a.active}<span class="badge live">Active</span>{/if}
            {#if a.plan?.label}<span class="badge">{a.plan.label}</span>{/if}
            {#if a.disabled}<span class="badge">Off</span>{/if}
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
    justify-content: space-between;
  }
  .who {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-size: 13px;
  }
  .badges {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
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
  .footer {
    display: flex;
    gap: 8px;
    margin-top: 16px;
  }
</style>
