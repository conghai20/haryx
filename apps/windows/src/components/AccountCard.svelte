<script lang="ts">
  // One account card, as macOS AccountCard / ActiveAccountCard. `live` is the account in use: a
  // wider card above the grid with a larger mascot and figure and a cyan "In use" by its name (cyan
  // is the live colour). An Off account (held out of rotation) steps back: a lighter fill, a pause
  // mark before its name, a dimmed mascot and Turn On in place of Switch. Its usage is still tracked,
  // so it keeps the full tide reading; "Off: held out of rotation" takes the note line.
  import { actions } from "../lib/actions";
  import { exhausted } from "../lib/handoff";
  import { store } from "../lib/store.svelte";
  import { FIVE_HOUR_S, HIGH_PCT, SEVEN_DAY_S, cardWeeklyPace, displayName, middleEllipsis, needsRelogin, paceLine, pctLabel, statusNote, tier } from "../lib/tide";
  import type { Account } from "../lib/types";
  import Mascot from "./Mascot.svelte";
  import TideStaff from "./TideStaff.svelte";
  import WeekTide from "./WeekTide.svelte";

  let {
    account: a,
    live = false,
    transitionName,
    selected,
    onselect,
    oncontextmenu,
  }: {
    account: Account;
    live?: boolean;
    /** Unique on the page: an account in two groups shows twice, and a repeated name stops the transition. */
    transitionName: string;
    selected: boolean;
    onselect: () => void;
    oncontextmenu: (e: MouseEvent) => void;
  } = $props();

  let off = $derived(!!a.disabled);
  let five = $derived(a.usage?.fiveHour);
  let week = $derived(a.usage?.sevenDay);
  let pace = $derived(paceLine(five, store.now));
  let weekPace = $derived(cardWeeklyPace(week, store.now));
  let running = $derived(store.runningFor(a));
  let note = $derived(statusNote(a));
</script>

<article style:view-transition-name={transitionName} class="card" class:live class:off class:selected class:limit={!off && exhausted(a)} aria-label={displayName(a)} {oncontextmenu}>
  <button class="hit" aria-label="Inspect {displayName(a)}" onclick={onselect}></button>
  <div class="identity">
    <Mascot account={a} size={live ? 96 : 72} />
    <div class="who">
      <div class="name-row">
        {#if off}
          <svg class="pause" viewBox="0 0 16 16" role="img" aria-label="Off">
            <circle cx="8" cy="8" r="7" />
            <rect x="5.5" y="4.75" width="1.75" height="6.5" rx="0.5" />
            <rect x="8.75" y="4.75" width="1.75" height="6.5" rx="0.5" />
          </svg>
        {/if}
        <strong class="truncate" class:secondary={off} title={displayName(a)}>{middleEllipsis(displayName(a), live ? 40 : 24)}</strong>
        {#if live}<span class="in-use"><span class="dot"></span>In use</span>{/if}
      </div>
      <span class="caption tertiary truncate">#{a.number} · {a.email}</span>
      <div class="badges">
        {#if a.plan?.label}<span class="badge">{a.plan.label}</span>{/if}
        {#if !off && exhausted(a)}<span class="badge">Limit</span>{/if}
        {#if running > 0}
          <span class="caption running"><span class="dot"></span> {running === 1 ? "running" : `${running} running`}</span>
        {/if}
      </div>
      {#if live}
        <div class="actions">
          {#if needsRelogin(a)}<button class="mini sign-in" onclick={() => actions.signIn(a)}>Sign In Again</button>{/if}
          <button class="mini" onclick={() => actions.runAs(a)}>Claude as this account</button>
        </div>
      {/if}
    </div>
  </div>

  <div class="reading">
    <div class="figure-row">
      <span
        class="display figure"
        class:warn={tier(five, FIVE_HOUR_S, store.now) === "warn"}
        class:limit={tier(five, FIVE_HOUR_S, store.now) === "limit"}>{pctLabel(five?.pct)}</span
      >
      <span class="caption secondary">5-hour</span>
      {#if five?.clock}<span class="caption secondary reset">resets {five.clock}</span>{/if}
    </div>
    <TideStaff window={five} windowS={FIVE_HOUR_S} now={store.now} height={5} label="5-hour usage" {off} />
    <div class="caption line">
      {#if pace && pace.text !== "on pace"}<span class:warn-text={pace.warn}>{pace.text}</span>{:else}&nbsp;{/if}
    </div>
    <div class="caption line">
      <span class:warn-text={(week?.pct ?? 0) >= HIGH_PCT}>Week {pctLabel(week?.pct)}</span>
      {#if week?.countdown}<span class="tertiary"> · resets in {week.countdown}</span>{/if}
    </div>
    <div class="week-tide"><WeekTide window={week} readings={store.weekReadings[a.number] ?? []} now={store.now} {off} /></div>
    <div class="caption line truncate" class:warn-text={weekPace?.warn} class:secondary={!weekPace?.warn} aria-label={weekPace ? `Weekly pace: ${weekPace.text}` : undefined}>
      {weekPace?.text ?? ""}&nbsp;
    </div>
    <div class="caption note" class:secondary={!note}>{note ?? (off ? "Off: held out of rotation" : "")}&nbsp;</div>
  </div>

  {#if !live}
    <div class="actions">
      {#if needsRelogin(a)}
        <button class="mini sign-in" onclick={() => actions.signIn(a)}>Sign In Again</button>
      {:else if off}
        <button class="mini" title="Put this account back in rotation" disabled={!!store.busy} onclick={() => actions.setDisabled(a, false)}>Turn On</button>
      {:else}
        <button class="mini" disabled={!!store.busy} onclick={() => actions.use(a)}>Switch</button>
      {/if}
      <button class="mini" onclick={() => actions.runAs(a)}>Claude as this account</button>
    </div>
  {/if}
</article>

<style>
  .week-tide {
    margin-top: 2px;
  }
  .card {
    position: relative;
    padding: 14px;
    border-radius: var(--radius-card);
    background: var(--fill);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .card.off {
    background: color-mix(in srgb, var(--fill) 55%, transparent);
  }
  .card.selected {
    background: var(--selected);
  }
  /* The live card: identity left, the reading right; stacked when narrow. */
  .card.live {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: minmax(260px, 420px) minmax(280px, 1fr);
    gap: 24px;
    padding: 18px;
  }
  @media (max-width: 760px) {
    .card.live {
      grid-template-columns: 1fr;
      gap: 14px;
    }
  }
  .card.off :global(canvas),
  .card.limit :global(canvas) {
    opacity: 0.55;
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
  .identity,
  .reading,
  .actions {
    position: relative;
  }
  .identity {
    display: flex;
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
  .live .who strong {
    font-size: 16px;
  }
  .name-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .pause {
    flex: none;
    width: 14px;
    height: 14px;
  }
  .pause circle {
    fill: var(--label-2);
  }
  .pause rect {
    fill: var(--surface);
  }
  .in-use {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-weight: 600;
    color: var(--cyan-text);
  }
  .in-use .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--cyan);
  }
  .badges {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    margin-top: 4px;
  }
  .running .dot {
    margin-right: 3px;
  }
  .figure-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 2px 0 4px;
  }
  .reset {
    margin-left: auto;
  }
  .figure {
    font-size: 28px;
  }
  .live .figure {
    font-size: 44px;
  }
  .figure.warn,
  .warn-text {
    color: var(--amber);
  }
  .figure.limit {
    color: var(--red);
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
  .note.secondary {
    color: var(--label-2);
  }
  .actions {
    margin-top: auto;
    padding-top: 6px;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .sign-in {
    background: var(--amber);
    color: white;
  }
</style>
