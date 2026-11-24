<script lang="ts">
  // `haryx ai wake`: the scheduled task, quiet hours and weekly cap, a per-account opt-in, and the
  // recent wake events. Every change reloads `ai wake status --json`.
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { store } from "../lib/store.svelte";
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
  let loadError = $state<string | null>(null);
  let quietOn = $state(false);
  let quietStart = $state("01:00");
  let quietEnd = $state("06:00");
  let cap = $state(90);

  let onCount = $derived(wake?.accounts.filter((a) => a.on).length ?? 0);
  let allOn = $derived(!!wake && wake.accounts.length > 0 && onCount === wake.accounts.length);
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
      loadError = String(e);
    }
  }

  async function change(label: string, args: string[], success?: string) {
    await store.run(label, ["ai", "wake", ...args], { success });
    await load();
  }

  const setOn = (ref: string, on: boolean) => change(on ? "Wake on" : "Wake off", [on ? "on" : "off", ref]);
  const setTask = (install: boolean) =>
    change(install ? "Installing the scheduled task" : "Removing the scheduled task", [install ? "install" : "uninstall"]);
  const saveSettings = () =>
    change(
      "Wake settings",
      ["config", ...(quietOn ? ["--quiet", `${quietStart}-${quietEnd}`] : ["--no-quiet"]), "--weekly-cap", String(cap)],
      "Wake settings saved",
    );

  function nameOf(row: WakeAccount): string {
    const a = store.accounts.find((x) => x.number === row.number);
    return a ? displayName(a) : row.alias || row.email;
  }

  function eventName(ref: string | null | undefined): string {
    const row = wake?.accounts.find((a) => String(a.number) === ref);
    return row ? nameOf(row) : ref ? `Account ${ref}` : "—";
  }

  function when(iso: string | null | undefined): string {
    if (!iso) return "—";
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" });
  }

  function result(r: string | null | undefined): string {
    if (!r) return "—";
    if (r === "woke") return "Woke";
    if (r === "not-due") return "Not due";
    const [kind, why] = r.split(":", 2);
    const reason = (why ?? "").replaceAll("-", " ");
    if (kind === "skip") return `Skipped: ${reason}`;
    if (kind === "failed") return `Failed: ${reason}`;
    return r;
  }

  onMount(load);
</script>

{#if wake}
  <section aria-labelledby="wake-task">
    <div class="row">
      <div>
        <h2 id="wake-task">Scheduled task</h2>
        <p class="caption">
          When an opted-in account's 5-hour window resets, one 1-token request starts the next window. Checks with
          nothing due read local files only.
        </p>
      </div>
      <span class="spacer"></span>
      {#if wake.agentInstalled}<span class="badge live">Every minute</span>{:else}<span class="badge">Off</span>{/if}
      <button disabled={!!store.busy} onclick={() => setTask(!wake?.agentInstalled)}>
        {wake.agentInstalled ? "Remove" : "Install"}
      </button>
    </div>
  </section>

  <div class="hairline"></div>

  <section aria-labelledby="wake-settings" class="settings">
    <h2 id="wake-settings">Settings</h2>
    <label class="line"><input type="checkbox" bind:checked={quietOn} /> Quiet hours</label>
    {#if quietOn}
      <div class="line indent">
        <label>From <input type="time" bind:value={quietStart} required /></label>
        <label>Until <input type="time" bind:value={quietEnd} required /></label>
      </div>
    {/if}
    <label class="line">
      Skip at weekly use
      <input type="range" min="50" max="100" step="5" bind:value={cap} aria-valuetext="{cap}%" />
      <span class="pct">{cap}%</span>
    </label>
    <div><button class="primary" disabled={!!store.busy || (quietOn && (!quietStart || !quietEnd))} onclick={saveSettings}>Save</button></div>
  </section>

  <div class="hairline"></div>

  <section aria-labelledby="wake-accounts">
    <div class="row">
      <h2 id="wake-accounts">Accounts</h2>
      <span class="caption">{onCount} of {wake.accounts.length} on</span>
      <span class="spacer"></span>
      <label class="line">
        <input
          type="checkbox"
          checked={allOn}
          disabled={wake.accounts.length === 0 || !!store.busy}
          onchange={(e) => setOn("all", e.currentTarget.checked)}
        />
        All accounts
      </label>
    </div>
    {#if wake.accounts.length === 0}
      <p class="secondary">No accounts yet.</p>
    {:else}
      <table>
        <thead class="caption tertiary">
          <tr><th>Wake</th><th>Account</th><th>Last wake</th><th>Next due</th><th>Last result</th></tr>
        </thead>
        <tbody>
          {#each wake.accounts as row (row.number)}
            <tr>
              <td>
                <input
                  type="checkbox"
                  checked={row.on}
                  disabled={!!store.busy}
                  aria-label="Wake {nameOf(row)}"
                  onchange={(e) => setOn(String(row.number), e.currentTarget.checked)}
                />
              </td>
              <td class="truncate" title={row.email}>{nameOf(row)}</td>
              <td class="secondary">{when(row.lastWakeAt)}</td>
              <td class="secondary">{when(row.nextDue)}</td>
              <td class:secondary={!row.lastResult?.startsWith("failed")} class:fail={row.lastResult?.startsWith("failed")}>
                {result(row.lastResult)}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if onCount > 0 && !wake.agentInstalled}
        <p class="caption">The scheduled task is off, so opted-in accounts are not checked. Install it above.</p>
      {/if}
    {/if}
  </section>

  <div class="hairline"></div>

  <section aria-labelledby="wake-events">
    <h2 id="wake-events">Recent events</h2>
    {#if events.length === 0}
      <p class="secondary">No wakes yet.</p>
    {:else}
      <ul>
        {#each events as ev, i (`${ev.at}-${i}`)}
          <li>
            <span class="tertiary">{new Date(ev.at * 1000).toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" })}</span>
            <span class="truncate">{eventName(ev.account)}</span>
            <span class="secondary">{result(ev.result)}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{:else if loadError}
  <div class="banner error" role="alert">
    Could not read wake status: {loadError}. Update haryx (Tools, Upgrade) if this version has no wake command.
  </div>
  <p><button onclick={load}>Try again</button></p>
{:else}
  <p class="secondary" role="status">Loading wake status…</p>
{/if}

<style>
  h2 {
    margin: 0 0 4px;
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
  .indent {
    padding-left: 22px;
    gap: 16px;
  }
  .pct {
    width: 40px;
    text-align: right;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    margin-top: 6px;
    table-layout: fixed;
  }
  th {
    text-align: left;
    font-weight: 400;
  }
  th:first-child,
  td:first-child {
    width: 44px;
  }
  td {
    padding: 5px 0;
    border-top: 1px solid var(--separator);
  }
  .fail {
    color: var(--red);
  }
  ul {
    list-style: none;
    margin: 4px 0 0;
    padding: 0;
  }
  li {
    display: grid;
    grid-template-columns: 130px 1fr auto;
    gap: 10px;
    padding: 4px 0;
  }
</style>
