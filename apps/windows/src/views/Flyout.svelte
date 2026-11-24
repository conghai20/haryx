<script lang="ts">
  // The tray flyout (the macOS menu bar panel): header, the active account's tide reading, the other
  // accounts as an almanac with "Use", the last auto-switch event, a banner, and the footer actions.
  import { actions } from "../lib/actions";
  import { api } from "../lib/api";
  import { store } from "../lib/store.svelte";
  import { bestTarget } from "../lib/handoff";
  import {
    FIVE_HOUR_S,
    HIGH_PCT,
    SEVEN_DAY_S,
    displayName,
    needsRelogin,
    paceLine,
    pctLabel,
    shortName,
    statusNote,
    tier,
  } from "../lib/tide";
  import Lockup from "../components/Lockup.svelte";
  import TideStaff from "../components/TideStaff.svelte";

  let active = $derived(store.active);
  let five = $derived(active?.usage?.fiveHour ?? null);
  let week = $derived(active?.usage?.sevenDay ?? null);
  let pace = $derived(paceLine(five, store.now));
  let running = $derived(active ? store.runningFor(active) : 0);
  let next = $derived(store.others.find((a) => !a.disabled) ?? null);
  let terminalOpen = $state(false);

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") api.hideFlyout();
  }
</script>

<svelte:window onkeydown={onKey} />

<main class="flyout">
  <header>
    <Lockup size={18} />
    <span class="caption auto" title="Auto-switch">
      {#if store.autoRunning}<span class="dot"></span> Auto-switch on{:else}Auto-switch off{/if}
    </span>
    <span class="spacer"></span>
    <button class="plain" title="Refresh" aria-label="Refresh" onclick={() => api.refresh()}>⟳</button>
    <button class="plain" title="Add the current Claude Code login" aria-label="Add account" onclick={() => actions.addCurrent()}>＋</button>
  </header>

  <div class="hairline"></div>

  {#if active}
    <section class="active" aria-label="Active account">
      <div class="row">
        <div class="name truncate" title={active.email}>
          <strong>{displayName(active)}</strong>
          {#if active.plan?.label}<span class="badge">{active.plan.label}</span>{/if}
        </div>
        {#if running > 0}
          <span class="caption running"><span class="dot"></span> {running} running</span>
        {/if}
      </div>
      <div class="reading">
        <span class="display big" class:warn={tier(five, FIVE_HOUR_S, store.now) === "warn"} class:limit={tier(five, FIVE_HOUR_S, store.now) === "limit"}>
          {pctLabel(five?.pct)}
        </span>
        <span class="clock display">{five?.clock ?? five?.countdown ?? ""}</span>
      </div>
      <TideStaff window={five} windowS={FIVE_HOUR_S} now={store.now} height={6} label="5-hour usage" />
      <div class="caption pace">
        {#if pace}<span class:warn-text={pace.warn}>{pace.text}</span>{/if}
        {#if five?.countdown}<span class="tertiary"> · resets in {five.countdown}</span>{/if}
      </div>
      <div class="caption week-line">
        <span class:warn-text={(week?.pct ?? 0) >= HIGH_PCT}>Week {pctLabel(week?.pct)}</span>
        {#if week?.countdown}<span class="tertiary"> · resets in {week.countdown}</span>{/if}
      </div>
      <TideStaff window={week} windowS={SEVEN_DAY_S} now={store.now} height={3} label="Weekly usage" />
      {#if statusNote(active)}
        <div class="caption note">{statusNote(active)}</div>
      {/if}
      {#if needsRelogin(active)}
        <button class="primary sign-in" onclick={() => actions.signIn(active)}>Sign In Again</button>
      {/if}
    </section>
  {:else if store.error}
    <div class="banner error" role="alert">{store.error}</div>
  {:else}
    <p class="secondary empty">No accounts yet. Sign in to Claude Code, then add the current login.</p>
  {/if}

  <div class="hairline"></div>

  <section class="almanac" aria-label="Next in line">
    <div class="grid head caption tertiary">
      <span>Next in line</span><span class="num">5h</span><span class="num">7d</span><span class="num">Resets</span><span></span>
    </div>
    <div class="rows">
      {#each store.others as a (a.number)}
        {@const f = a.usage?.fiveHour}
        {@const w = a.usage?.sevenDay}
        <div class="grid row-item" class:off={a.disabled}>
          <span class="truncate" title={a.email}>
            {displayName(a)}
            {#if a.plan?.label}<span class="caption tertiary">{a.plan.label}</span>{/if}
          </span>
          <span class="num" class:warn-text={(f?.pct ?? 0) >= HIGH_PCT}>{pctLabel(f?.pct)}</span>
          <span class="num" class:warn-text={(w?.pct ?? 0) >= HIGH_PCT}>{pctLabel(w?.pct)}</span>
          <span class="num tertiary">{f?.clock ?? f?.countdown ?? ""}</span>
          <button class="mini" disabled={!!store.busy} onclick={() => actions.use(a)} aria-label="Use {displayName(a)}">Use</button>
        </div>
      {:else}
        <p class="caption tertiary">No other accounts.</p>
      {/each}
    </div>
  </section>

  {#if store.stuck.length > 0}
    {@const target = bestTarget(store.accounts, store.stuck[0]?.account ?? null)}
    {#if target}
      <div class="update caption stuck-row">
        <span class="truncate">
          {store.stuck.length === 1 ? "1 session" : `${store.stuck.length} sessions`} stopped on account
          {[...new Set(store.stuck.map((s) => s.account))].join(", ")}
        </span>
        <button
          onclick={() => store.stuck.forEach((s) => actions.continueSession(s.session.pid, target.number, s.session.cwd))}
        >Continue on {target.number}</button>
      </div>
    {/if}
  {/if}
  {#if store.update?.state.phase === "ready"}
    <div class="update caption">
      <span>HaryX {store.update.state.readyVersion} is ready</span>
      <button onclick={() => api.updateRestart().catch((e) => store.flash(String(e), true))}>Restart</button>
    </div>
  {:else if store.update?.available}
    <div class="update caption">
      <span>HaryX {store.update.available} is available</span>
      <button onclick={() => api.openDashboard("about")}>Details</button>
    </div>
  {/if}
  {#if store.lastAutoEvent}
    <p class="caption event truncate" title={store.lastAutoEvent.summary}>{store.lastAutoEvent.summary}</p>
  {/if}
  {#if store.message}
    <div class="banner" class:error={store.message.error} role="status">{store.message.text}</div>
  {:else if store.busy}
    <div class="banner" role="status">{store.busy}…</div>
  {/if}

  <footer>
    <button class="primary switch" disabled={!next || !!store.busy} onclick={() => actions.next()}>
      {next ? `Switch to ${shortName(next)}` : "Switch"}
    </button>
    <button title="Open the dashboard" aria-label="Open the dashboard" onclick={() => api.openDashboard()}>▦</button>
    <div class="split">
      <button onclick={() => (active ? actions.runAs(active) : actions.terminal({ kind: "claude" }))}>Terminal</button>
      <button aria-label="More terminal actions" aria-expanded={terminalOpen} onclick={() => (terminalOpen = !terminalOpen)}>▾</button>
      {#if terminalOpen}
        <div class="menu" role="menu">
          <button role="menuitem" onclick={() => actions.terminal({ kind: "shell" })}>New shell</button>
          <button role="menuitem" onclick={() => actions.terminal({ kind: "claude" })}>Claude (active account)</button>
          {#each store.accounts as a (a.number)}
            <button role="menuitem" onclick={() => actions.runAs(a)}>Claude as {displayName(a)}</button>
          {/each}
        </div>
      {/if}
    </div>
  </footer>
</main>

<style>
  .flyout {
    height: 100%;
    padding: 14px;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  header,
  footer,
  .row,
  .reading {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .spacer {
    flex: 1;
  }
  .auto .dot,
  .running .dot {
    margin-right: 4px;
  }
  .name {
    flex: 1;
    font-size: 13px;
  }
  .name .badge {
    margin-left: 6px;
  }
  .reading {
    justify-content: space-between;
    margin: 6px 0 8px;
  }
  .big {
    font-size: 44px;
  }
  .big.warn,
  .warn-text {
    color: var(--amber);
  }
  .big.limit {
    color: var(--red);
  }
  .clock {
    font-size: 18px;
    font-weight: 500;
    color: var(--label-2);
  }
  .pace,
  .week-line {
    margin-top: 6px;
    margin-bottom: 4px;
  }
  .note {
    margin-top: 8px;
    color: var(--amber);
  }
  .sign-in {
    margin-top: 8px;
    background: var(--amber);
  }
  .empty {
    margin: 8px 0;
  }
  .almanac {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .rows {
    overflow-y: auto;
    max-height: 300px;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 40px 40px 70px 52px;
    column-gap: 10px;
    align-items: center;
    padding: 5px 8px;
  }
  .row-item {
    border-radius: var(--radius-control);
  }
  .row-item:hover {
    background: color-mix(in srgb, var(--fill-strong) 50%, transparent);
  }
  .row-item.off {
    opacity: 0.5;
  }
  .num {
    text-align: right;
  }
  .update {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin: 8px 0 0;
    font-weight: 600;
    color: var(--accent, inherit);
  }
  .stuck-row {
    color: var(--orange, #c75000);
  }
  .event {
    margin: 8px 0 0;
    color: var(--label-2);
  }
  .banner {
    margin-top: 8px;
  }
  footer {
    margin-top: 12px;
  }
  .switch {
    flex: 1;
    padding: 8px;
  }
  .split {
    position: relative;
    display: flex;
    gap: 1px;
  }
  .menu {
    position: absolute;
    right: 0;
    bottom: 110%;
    min-width: 220px;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-banner);
    padding: 4px;
    z-index: 2;
  }
  .menu button {
    text-align: left;
    background: transparent;
  }
  .menu button:hover {
    background: var(--fill);
  }
</style>
