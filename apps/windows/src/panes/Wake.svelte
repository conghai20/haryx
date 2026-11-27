<script lang="ts">
  // `haryx ai wake`: the background check (a scheduled task), quiet hours and weekly cap, a
  // per-account opt-in, and the recent wake events. Every change saves at once and reloads
  // `ai wake status --json`.
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { describeError } from "../lib/errors";
  import { store } from "../lib/store.svelte";
  import { relativeTime, wakeResultText } from "../lib/text";
  import { displayName } from "../lib/tide";

  interface WakeAccount {
    number: number;
    email: string;
    alias?: string | null;
    on: boolean;
    lastWakeAt?: string | null;
    lastCheckAt?: string | null;
    nextDue?: string | null;
    lastResult?: string | null;
  }
  interface WakeEvent {
    at: number;
    account?: string | null;
    result: string;
  }
  interface WakeStatus {
    accounts: WakeAccount[];
    quietHours?: string | null;
    weeklyCapPct: number;
    agentInstalled: boolean;
    events?: WakeEvent[] | null;
  }

  const EVENTS_SHOWN = 20;
  const QUIET_RE = /^(\d{1,2}:\d{2})-(\d{1,2}:\d{2})$/;

  let wake = $state<WakeStatus | null>(null);
  let loadError = $state<{ text: string; detail: string } | null>(null);
  let quietOn = $state(false);
  let quietStart = $state("01:00");
  let quietEnd = $state("06:00");
  let cap = $state(90);

  let onCount = $derived(wake?.accounts.filter((a) => a.on).length ?? 0);
  let allOn = $derived(!!wake && wake.accounts.length > 0 && onCount === wake.accounts.length);
  /** Some but not all on: the All accounts box shows a mixed state. */
  let someOn = $derived(onCount > 0 && !allOn);
  let events = $derived([...(wake?.events ?? [])].sort((a, b) => b.at - a.at).slice(0, EVENTS_SHOWN));

  async function load() {
    try {
      const w = await api.json<WakeStatus>(["ai", "wake", "status", "--json"]);
      wake = w;
      loadError = null;
      cap = Math.round(w.weeklyCapPct);
      const m = w.quietHours ? QUIET_RE.exec(w.quietHours) : null;
      quietOn = !!m;
      if (m) {
        quietStart = (m[1] ?? "01:00").padStart(5, "0");
        quietEnd = (m[2] ?? "06:00").padStart(5, "0");
      }
    } catch (e) {
      loadError = describeError(e, "Couldn't read wake status");
    }
  }

  async function change(label: string, args: string[], success?: string) {
    await store.run(label, ["ai", "wake", ...args], { success });
    await load();
  }

  const setOn = (ref: string, on: boolean) => change(on ? "Wake on" : "Wake off", [on ? "on" : "off", ref]);
  const setTask = (install: boolean) =>
    change(install ? "Installing the scheduled task" : "Removing the scheduled task", [install ? "install" : "uninstall"]);
  /** Saves quiet hours and the weekly cap as soon as either changes; nothing when they match what is stored. */
  function saveSettings() {
    if (!wake || (quietOn && (!quietStart || !quietEnd))) return;
    const quiet = quietOn ? `${quietStart}-${quietEnd}` : null;
    if (quiet === (wake.quietHours ?? null) && cap === Math.round(wake.weeklyCapPct)) return;
    void change(
      "Saving wake settings",
      ["config", ...(quiet ? ["--quiet", quiet] : ["--no-quiet"]), "--weekly-cap", String(cap)],
      "Wake settings saved",
    );
  }

  function nameOf(row: WakeAccount): string {
    const a = store.accounts.find((x) => x.number === row.number);
    return a ? displayName(a) : row.alias || row.email;
  }

  function eventName(ref: string | null | undefined): string {
    const row = wake?.accounts.find((a) => String(a.number) === ref);
    return row ? nameOf(row) : ref ? `Account ${ref}` : "—";
  }

  const time = (iso: string | null | undefined) => (iso ? new Date(iso).getTime() : NaN);

  function when(iso: string | null | undefined): string {
    const t = time(iso);
    return Number.isNaN(t) ? "—" : new Date(t).toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" });
  }

  /** "Woke 2 hr ago · next in 34 min", relative; the result follows it. */
  function statusLine(row: WakeAccount): string {
    const last = Number.isNaN(time(row.lastWakeAt)) ? "No wake yet" : `Woke ${relativeTime(time(row.lastWakeAt), store.now)}`;
    const next = Number.isNaN(time(row.nextDue)) ? "" : ` · next ${relativeTime(time(row.nextDue), store.now)}`;
    return last + next;
  }

  const absoluteLine = (row: WakeAccount) => `Last wake ${when(row.lastWakeAt)} · Next due ${when(row.nextDue)}`;

  /** The All accounts box: mixed when some are on. One click sends a single `wake on|off all`: on from
      off or mixed (as macOS), off when all are on. */
  function mixed(node: HTMLInputElement, value: boolean) {
    node.indeterminate = value;
    return { update: (v: boolean) => (node.indeterminate = v) };
  }

  onMount(load);
</script>

{#if wake}
  <section aria-labelledby="wake-task">
    <div class="row">
      <h2 id="wake-task">Background check</h2>
      <span class="spacer"></span>
      <span class="caption">{wake.agentInstalled ? "Every minute" : "Off"}</span>
      <button disabled={!!store.busy} onclick={() => setTask(!wake?.agentInstalled)}>
        {wake.agentInstalled ? "Remove" : "Install"}
      </button>
    </div>
    <p class="caption">
      When an opted-in account's 5-hour window resets, one 1-token request starts the next window. Checks with nothing
      due read local files only.
    </p>
  </section>

  <section aria-labelledby="wake-settings" class="settings">
    <h2 id="wake-settings">Settings</h2>
    <div class="line between">
      <label for="quiet">Quiet hours</label>
      <input
        id="quiet"
        type="checkbox"
        role="switch"
        class="switch"
        bind:checked={quietOn}
        disabled={!!store.busy}
        onchange={saveSettings}
      />
    </div>
    {#if quietOn}
      <div class="line indent">
        <label>From <input type="time" bind:value={quietStart} required onchange={saveSettings} /></label>
        <label>Until <input type="time" bind:value={quietEnd} required onchange={saveSettings} /></label>
      </div>
    {/if}
    <div class="line between">
      <label for="cap">Skip at weekly use</label>
      <span class="line">
        <input
          id="cap"
          type="range"
          min="50"
          max="100"
          step="5"
          bind:value={cap}
          aria-valuetext="{cap}%"
          onchange={saveSettings}
        />
        <span class="pct">{cap}%</span>
      </span>
    </div>
  </section>

  <section aria-labelledby="wake-accounts">
    <h2 id="wake-accounts">Accounts</h2>
    {#if wake.accounts.length === 0}
      <p class="secondary">No accounts yet.</p>
    {:else}
      <ul class="accounts">
        <li class="all">
          <div class="who">
            <label for="wake-all"><strong>All accounts</strong></label>
            <span class="caption">{onCount} of {wake.accounts.length} on</span>
          </div>
          <input
            id="wake-all"
            type="checkbox"
            role="switch"
            class="switch"
            checked={allOn}
            use:mixed={someOn}
            aria-checked={someOn ? "mixed" : allOn}
            disabled={!!store.busy}
            onchange={() => setOn("all", !allOn)}
          />
        </li>
        {#each wake.accounts as row (row.number)}
          {@const failed = row.lastResult?.startsWith("failed") ?? false}
          <li>
            <div class="who" title={absoluteLine(row)}>
              <label for="wake-{row.number}" class="truncate">{nameOf(row)}</label>
              <span class="caption">
                {statusLine(row)}{#if row.lastResult}
                  · <span class:fail={failed}>{wakeResultText(row.lastResult)}</span>{/if}
              </span>
            </div>
            <input
              id="wake-{row.number}"
              type="checkbox"
              role="switch"
              class="switch"
              checked={row.on}
              disabled={!!store.busy}
              aria-label="Wake {nameOf(row)}"
              onchange={(e) => setOn(String(row.number), e.currentTarget.checked)}
            />
          </li>
        {/each}
      </ul>
      {#if onCount > 0 && !wake.agentInstalled}
        <p class="caption">The background check is off, so opted-in accounts are not checked. Install it above.</p>
      {/if}
    {/if}
  </section>

  <section aria-labelledby="wake-events">
    <h2 id="wake-events">Recent events</h2>
    {#if events.length === 0}
      <p class="secondary">No wakes yet.</p>
    {:else}
      <ul class="events">
        {#each events as ev, i (`${ev.at}-${i}`)}
          {@const at = ev.at * 1000}
          <li>
            <span class="tertiary" title={new Date(at).toLocaleString()}>{relativeTime(at, store.now)}</span>
            <span class="truncate">{eventName(ev.account)}</span>
            <span class:secondary={!ev.result.startsWith("failed")} class:fail={ev.result.startsWith("failed")}
              >{wakeResultText(ev.result)}</span
            >
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{:else if loadError}
  <div class="banner error" role="alert" title={loadError.detail}>
    {loadError.text} If this haryx has no wake command, upgrade it in Backup & maintenance.
  </div>
  <p><button onclick={load}>Try again</button></p>
{:else}
  <p class="secondary" role="status">Loading wake status…</p>
{/if}

<style>
  section {
    margin-bottom: 20px;
  }
  h2 {
    margin: 0 0 6px;
    font-size: 13px;
    font-weight: 600;
  }
  section p {
    margin: 4px 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .spacer {
    flex: 1;
  }
  .settings {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .between {
    justify-content: space-between;
  }
  .indent {
    padding-left: 16px;
    gap: 16px;
  }
  .pct {
    width: 40px;
    text-align: right;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .accounts li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 0;
    border-top: 1px solid var(--separator);
  }
  .who {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .fail {
    color: var(--red);
  }
  .events li {
    display: grid;
    grid-template-columns: 110px 1fr auto;
    gap: 10px;
    padding: 5px 0;
    border-top: 1px solid var(--separator);
  }
</style>
