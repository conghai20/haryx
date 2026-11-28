<script lang="ts">
  // The tray flyout (the macOS menu bar panel): header, the active account's tide reading, the other
  // accounts as an almanac with "Use" and a per-row action menu, the last auto-switch event, a banner,
  // and the footer actions.
  import { actions } from "../lib/actions";
  import { api } from "../lib/api";
  import { store } from "../lib/store.svelte";
  import ClaudeSetupBanner from "../components/ClaudeSetupBanner.svelte";
  import { accountGroups, groupNames, groupSections, groupsInUse, isInGroup, listSections } from "../lib/groups";
  import { bestTarget, exhausted, resting } from "../lib/handoff";
  import {
    FIVE_HOUR_S,
    HIGH_PCT,
    SEVEN_DAY_S,
    displayName,
    middleEllipsis,
    needsRelogin,
    paceLine,
    pctLabel,
    plansVary,
    rowName,
    shortName,
    statusNote,
    tier,
    canRelogin,
    weekIsTighter,
  } from "../lib/tide";
  import type { Account } from "../lib/types";
  import Icon from "../components/Icon.svelte";
  import Lockup from "../components/Lockup.svelte";
  import Mascot from "../components/Mascot.svelte";
  import TideStaff from "../components/TideStaff.svelte";

  let active = $derived(store.active);
  // The 5-hour window leads; the week sits under it and turns amber when it is the tighter limit.
  let five = $derived(active?.usage?.fiveHour ?? null);
  let week = $derived(active?.usage?.sevenDay ?? null);
  let fiveTier = $derived(tier(five, FIVE_HOUR_S, store.now));
  let weekWarn = $derived((week?.pct ?? 0) >= HIGH_PCT || weekIsTighter(active?.usage));
  let pace = $derived(paceLine(five, store.now));
  // Each group once with switch order inside it; accounts held out of rotation are never next, so
  // they fold into one line at the bottom (macOS NextInLine).
  let sections = $derived(listSections(store.others.filter((a) => !a.disabled), active ?? null));
  let heldOut = $derived(store.others.filter((a) => a.disabled));
  let showHeldOut = $state(false);
  let showsPlan = $derived(plansVary(store.accounts));
  /** Middle-cut row names: local parts often differ only at the end ("…claude03"). */
  // ponytail: a character count, not a measured width; measure the cell if names still clip.
  const ROW_NAME_CHARS = 18;
  let running = $derived(active ? store.runningFor(active) : 0);
  // The best account, not the first row: every row already has its own Use (macOS QuickActions).
  let best = $derived(bestTarget(store.accounts, active?.number ?? null));
  let terminalOpen = $state(false);
  /** The account row whose action menu is open. */
  let rowMenu = $state<number | null>(null);

  function onKey(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    if (rowMenu != null || terminalOpen) {
      rowMenu = null;
      terminalOpen = false;
    } else api.hideFlyout();
  }

  function rowAction(run: () => Promise<unknown>) {
    rowMenu = null;
    run().catch((e) => store.flash(String(e), true));
  }

  function terminalAction(run: () => Promise<unknown>) {
    terminalOpen = false;
    run().catch((e) => store.flash(String(e), true));
  }
</script>

<svelte:window onkeydown={onKey} />

{#snippet accountRow(a: Account)}
  {@const f = a.usage?.fiveHour}
  {@const w = a.usage?.sevenDay}
  <!-- Only an account held out of rotation is dimmed; one at its limit keeps its red figure. -->
  <div class="grid row-item" class:off={a.disabled}>
    <button
      class="identity plain"
      title={a.email}
      aria-haspopup="menu"
      aria-expanded={rowMenu === a.number}
      onclick={() => (rowMenu = rowMenu === a.number ? null : a.number)}
    >
      <Mascot account={a} size={18} />
      <span class="truncate">
        {middleEllipsis(rowName(a, store.accounts), ROW_NAME_CHARS)}
        {#if showsPlan && a.plan?.label}<span class="caption tertiary">{a.plan.label}</span>{/if}
      </span>
    </button>
    <span class="num" class:warn-text={(f?.pct ?? 0) >= HIGH_PCT}>{pctLabel(f?.pct)}</span>
    <span class="num" class:warn-text={(w?.pct ?? 0) >= HIGH_PCT}>{pctLabel(w?.pct)}</span>
    <span class="num tertiary">{f?.clock ?? f?.countdown ?? ""}</span>
    <button
      class="mini"
      disabled={!!store.busy || resting(a)}
      title={exhausted(a) ? `${displayName(a)} is at its limit` : undefined}
      onclick={() => actions.use(a)}
      aria-label="Use {displayName(a)}">Use</button
    >
  </div>
  {#if rowMenu === a.number}
    <!-- macOS AccountActionMenu: Info, Sign In Again, Switch, Force Switch, Group, Remove. -->
    <div class="menu row-menu" role="menu" aria-label="{displayName(a)} actions">
      <button role="menuitem" onclick={() => rowAction(() => api.openDashboard("accounts", a.number))}>Info</button>
      {#if canRelogin(a)}
        <div class="sep" role="separator"></div>
        <button role="menuitem" onclick={() => rowAction(() => actions.signIn(a))}>Sign In Again…</button>
      {/if}
      <div class="sep" role="separator"></div>
      <button role="menuitem" disabled={a.active || !!store.busy} onclick={() => rowAction(() => actions.use(a))}>Switch</button>
      <!-- Named and confirmed as in the inspector: it skips backing up the current login. -->
      <button role="menuitem" disabled={!!store.busy} onclick={() => rowAction(() => actions.forceUse(a))}
        >Switch Without Saving Current Login…</button
      >
      <div class="sep" role="separator"></div>
      <p class="caption tertiary menu-head" aria-hidden="true">Groups</p>
      {#each groupNames(store.accounts) as name (name)}
        {@const member = isInGroup(a, name)}
        <button
          role="menuitemcheckbox"
          aria-checked={member}
          disabled={!!store.busy}
          onclick={() => rowAction(() => (member ? actions.removeFromGroup(a, name) : actions.addToGroup(a, name)))}
          >{member ? "✓ " : ""}{name}</button
        >
      {/each}
      <!-- The flyout has no room for the New Group dialog: it opens the dashboard on this account,
           whose details panel has its Groups field. -->
      <button role="menuitem" onclick={() => rowAction(() => api.openDashboard("accounts", a.number))}>New Group…</button>
      {#if accountGroups(a).length > 1}
        <button role="menuitem" disabled={!!store.busy} onclick={() => rowAction(() => actions.clearGroups(a))}>Remove from All Groups</button>
      {/if}
      <div class="sep" role="separator"></div>
      <button role="menuitem" disabled={!!store.busy} onclick={() => rowAction(() => actions.remove(a))}>Remove…</button>
    </div>
  {/if}
{/snippet}

<main class="flyout">
  <header>
    <Lockup size={18} />
    <!-- A switch, as on macOS: it reads as a control and shows its state. -->
    <button
      class="plain caption auto"
      class:on={store.autoRunning}
      role="switch"
      aria-checked={store.autoRunning}
      title={store.autoRunning ? "Stop auto-switch" : "Start auto-switch"}
      onclick={() => store.toggleAuto()}
    ><span class="track" aria-hidden="true"><span class="knob"></span></span>Auto-switch</button>
    <span class="spacer"></span>
    <button class="plain" title="Refresh" aria-label="Refresh" onclick={() => api.refresh()}>⟳</button>
    <button class="plain" title="Add the current Claude Code login" aria-label="Add account" onclick={() => actions.addCurrent()}>＋</button>
  </header>

  <div class="hairline"></div>
  <ClaudeSetupBanner compact />

  {#if active}
    <section class="active" aria-label="Active account">
      <div class="row">
        <Mascot account={active} size={36} />
        <div class="name truncate" title={active.email}>
          <strong>{displayName(active)}</strong>
          {#if active.plan?.label}<span class="badge">{active.plan.label}</span>{/if}
        </div>
        {#if running > 0}
          <span class="caption running"><span class="dot"></span> {running} running</span>
        {/if}
      </div>
      <div class="reading">
        <span class="display big" class:warn={fiveTier === "warn"} class:limit={fiveTier === "limit"}>
          {pctLabel(five?.pct)}
        </span>
        <span class="secondary figure">of 5-hour window</span>
        <span class="spacer"></span>
        {#if five?.clock ?? five?.countdown}
          <span class="clock display">{five?.clock ?? five?.countdown}</span>
        {:else}
          <!-- A window with no reset has not started. -->
          <span class="caption secondary">starts on first use</span>
        {/if}
      </div>
      <TideStaff window={five} windowS={FIVE_HOUR_S} now={store.now} height={6} label="5-hour usage" />
      <div class="caption pace">
        {#if pace}<span class:warn-text={pace.warn}>{pace.text}</span>{/if}
        {#if five?.countdown}<span class="tertiary"> · resets in {five.countdown}</span>{/if}
      </div>
      <div class="caption week-line">
        <span class:warn-text={weekWarn}>Week {pctLabel(week?.pct)}</span>
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
      {#each sections as section (section.key)}
        {#if groupsInUse(store.accounts)}
          <p class="caption tertiary group-row" role="heading" aria-level="3">{section.name ?? "Ungrouped"}</p>
        {/if}
        {#each section.accounts as a (a.number)}{@render accountRow(a)}{/each}
      {/each}
      {#if heldOut.length > 0}
        <button
          class="plain caption tertiary held-toggle"
          aria-expanded={showHeldOut}
          title="Accounts held out of rotation: auto-switch and Switch never pick them"
          onclick={() => (showHeldOut = !showHeldOut)}
        >
          <span class="chevron" class:open={showHeldOut} aria-hidden="true"><Icon name="chevronRight" size={12} /></span>
          Held out ({heldOut.length})
        </button>
        {#if showHeldOut}
          {#each heldOut as a (a.number)}{@render accountRow(a)}{/each}
        {/if}
      {/if}
      {#if store.others.length === 0}
        <p class="caption tertiary">No other accounts.</p>
      {/if}
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
    <button
      class="primary switch"
      disabled={!!store.busy}
      title={best ? `Switch to ${best.email}, the account with most headroom` : "Switch to the account with most headroom"}
      onclick={() => (best ? actions.use(best) : actions.next())}
    >
      {best ? `Switch to ${shortName(best)}` : "Switch"}
    </button>
    <button title="Open the dashboard" aria-label="Open the dashboard" onclick={() => api.openDashboard()}>▦</button>
    <div class="split">
      <button onclick={() => (active ? actions.runAs(active) : actions.terminal({ kind: "claude" }))}>Terminal</button>
      <button aria-label="More terminal actions" aria-expanded={terminalOpen} onclick={() => (terminalOpen = !terminalOpen)}>▾</button>
      {#if terminalOpen}
        <div class="menu" role="menu">
          <button role="menuitem" onclick={() => actions.terminal({ kind: "shell" })}>New shell</button>
          <button role="menuitem" onclick={() => actions.terminal({ kind: "claude" })}>Claude (active account)</button>
          {#if groupsInUse(store.accounts)}
            {#each groupSections(store.accounts) as section (section.key)}
              <p class="caption tertiary menu-head">{section.name ?? "Ungrouped"}</p>
              {#each section.accounts as a (a.number)}
                <button role="menuitem" onclick={() => actions.runAs(a)}>Claude as {displayName(a)}</button>
              {/each}
            {/each}
          {:else}
            {#each store.accounts as a (a.number)}
              <button role="menuitem" onclick={() => actions.runAs(a)}>Claude as {displayName(a)}</button>
            {/each}
          {/if}
          <button role="menuitem" onclick={() => terminalAction(() => actions.runInFolder(null))}>Claude in folder…</button>
          {#if store.mappings.length > 0}
            <div class="sep" role="separator"></div>
            <p class="caption tertiary menu-head">Mapped folders</p>
            {#each store.mappings as m (m.path)}
              <button role="menuitem" class="truncate" title={m.path} onclick={() => actions.terminal({ kind: "run-mapped" }, m.path)}>{m.path}</button>
            {/each}
          {/if}
          <div class="sep" role="separator"></div>
          <button role="menuitem" onclick={() => terminalAction(() => actions.signIn(null))}>Sign In New Account…</button>
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
  .running .dot {
    margin-right: 4px;
  }
  .auto {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--label-2);
    font-weight: 500;
  }
  .auto.on {
    color: var(--cyan-text);
  }
  .track {
    width: 22px;
    height: 12px;
    padding: 1px;
    border-radius: 6px;
    background: var(--track);
    display: inline-flex;
  }
  .auto.on .track {
    background: var(--cyan-ink);
    justify-content: flex-end;
  }
  .knob {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #fff;
  }
  .name {
    flex: 1;
    font-size: 13px;
  }
  .name .badge {
    margin-left: 6px;
  }
  .reading {
    align-items: baseline;
    margin: 6px 0 8px;
  }
  .figure {
    font-size: 12px;
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
  .group-row {
    margin: 6px 8px 0;
    font-weight: 500;
  }
  .held-toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    margin: 8px 0 2px;
    padding: 2px 8px;
    font-weight: 500;
  }
  .chevron {
    display: inline-flex;
    transition: transform 120ms ease-out;
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  @media (prefers-reduced-motion: reduce) {
    .chevron {
      transition: none;
    }
  }
  .row-item:hover {
    background: color-mix(in srgb, var(--fill-strong) 50%, transparent);
  }
  .identity {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
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
  .menu button:hover:not(:disabled) {
    background: var(--fill);
  }
  .row-menu {
    position: static;
    margin: 2px 8px 6px 32px;
    min-width: 0;
  }
  .sep {
    height: 1px;
    margin: 3px 4px;
    background: var(--separator);
  }
  .menu-head {
    margin: 2px 8px;
  }
  .menu {
    max-height: 420px;
    overflow-y: auto;
  }
  button.identity {
    padding: 0;
    text-align: left;
    color: inherit;
    font: inherit;
  }
</style>
