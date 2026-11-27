<script lang="ts">
  // Usage › The week: will the accounts auto-switch can use carry you to their resets? The CLI's
  // rotation verdict as one line, then an almanac row per account in switch order: name, Week Tide,
  // weekly %, pace and reset. Works without usage history (the tides then draw averages). macOS:
  // `WeekSection.swift`.
  import { revealAccount } from "../lib/nav";
  import { store } from "../lib/store.svelte";
  import {
    HIGH_PCT,
    cardWeeklyPace,
    displayName,
    middleEllipsis,
    needsRelogin,
    pctLabel,
    rotationDetail,
    rotationHeadline,
    runOutLabel,
  } from "../lib/tide";
  import type { Account } from "../lib/types";
  import WeekTide from "./WeekTide.svelte";

  let rotation = $derived(store.list?.rotation ?? null);
  /** Subscription accounts in switch order; API-key accounts have no week. */
  let rows = $derived(store.accounts.filter((a) => a.usageStatus !== "api_key"));

  function name(slot: number): string {
    const a = store.accounts.find((x) => x.number === slot);
    return a ? displayName(a) : `Account ${slot}`;
  }

  /** Why a row sits outside the verdict. */
  function note(a: Account): string | null {
    if (a.disabled) return "Held out of rotation";
    if (needsRelogin(a)) return "Signed out";
    if (rotation && !rotation.accounts.includes(a.number) && a.usage?.sevenDay) return "Not used by auto-switch";
    return null;
  }

  function paceText(a: Account): string {
    const pace = cardWeeklyPace(a.usage?.sevenDay, store.now);
    if (pace) return pace.text;
    return a.usage?.sevenDay ? "Pace from tomorrow" : "No reading yet";
  }
</script>

<section class="week" aria-labelledby="week-verdict">
  {#if rotation}
    <h2 id="week-verdict" class="verdict" class:warn-text={!rotation.lasts}>{rotationHeadline(rotation)}</h2>
    <p class="secondary">{rotationDetail(rotation)}</p>
    {#if rotation.unknown.length}
      <p class="caption secondary">Not counted yet, no weekly reading: {rotation.unknown.map(name).join(", ")}.</p>
    {/if}
  {:else}
    <h2 id="week-verdict" class="verdict">The week</h2>
    <p class="secondary">The verdict appears once an account auto-switch can use has a weekly reading.</p>
  {/if}

  <ul class="rows">
    {#each rows as a (a.number)}
      {@const week = a.usage?.sevenDay}
      {@const pace = cardWeeklyPace(week, store.now)}
      {@const why = note(a)}
      <li>
        <button class="row plain" onclick={() => revealAccount(a.number)} aria-label="{displayName(a)}: week {pctLabel(week?.pct)}, {paceText(a)}. Open details">
          <span class="who">
            <span class="name" class:secondary={a.disabled} title={displayName(a)}>{middleEllipsis(displayName(a), 30)}</span>
            {#if why}<span class="caption secondary">{why}</span>{/if}
          </span>
          <span class="tide"><WeekTide window={week} readings={store.weekReadings[a.number] ?? []} now={store.now} height={22} off={!!a.disabled} /></span>
          <span class="pct" class:warn-text={(week?.pct ?? 0) >= HIGH_PCT}>{pctLabel(week?.pct)}</span>
          <span class="pace truncate" class:warn-text={pace?.warn} class:secondary={!pace?.warn}>{paceText(a)}</span>
          <span class="reset secondary">{week?.resetsAt ? `resets ${runOutLabel(week.resetsAt) ?? ""}` : ""}</span>
        </button>
      </li>
    {/each}
  </ul>
</section>

<style>
  .week {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .verdict {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
  }
  p {
    margin: 0;
  }
  .rows {
    list-style: none;
    margin: 12px 0 0;
    padding: 0;
  }
  .rows li + li {
    border-top: 1px solid var(--separator);
  }
  .row {
    display: grid;
    grid-template-columns: 230px minmax(160px, 420px) 44px minmax(0, 1fr) auto;
    align-items: center;
    gap: 16px;
    width: 100%;
    padding: 8px 0;
    text-align: left;
    font: inherit;
    color: inherit;
  }
  .row:hover .name {
    text-decoration: underline;
  }
  .who {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .warn-text {
    color: var(--amber);
  }
  .pct {
    text-align: right;
    font-variant-numeric: tabular-nums;
    font-weight: 500;
  }
  .pace,
  .reset {
    font-variant-numeric: tabular-nums;
  }
  .reset {
    text-align: right;
    white-space: nowrap;
  }
  @media (max-width: 900px) {
    .row {
      grid-template-columns: minmax(0, 1fr) 44px;
    }
    .tide {
      grid-column: 1 / -1;
      grid-row: 2;
    }
    .pace,
    .reset {
      grid-column: 1 / -1;
      text-align: left;
    }
  }
</style>
