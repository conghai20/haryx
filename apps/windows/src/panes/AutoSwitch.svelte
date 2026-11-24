<script lang="ts">
  // Auto-switch: start or stop the `haryx auto --json` loop, run one tick, and read its live log.
  import { api } from "../lib/api";
  import { store } from "../lib/store.svelte";
  import type { RunResult } from "../lib/types";

  let log = $derived([...store.autoLog].reverse());
  let hidePolls = $state(true);
  let shown = $derived(hidePolls ? log.filter((e) => e.kind !== "poll") : log);

  async function runOnce() {
    store.busy = "Checking";
    try {
      const args = ["auto", "--once", "--json", ...(store.prefs?.autoSwitchDryRun ? ["--dry-run"] : [])];
      const r: RunResult = await api.haryx(args);
      const outcome =
        r.status === 0 ? "Switched" : r.status === 2 ? "No action needed" : r.status === 3 ? "Blocked: no viable account" : `Error: ${r.message}`;
      store.flash(outcome, r.status === 1);
    } catch (e) {
      store.flash(String(e), true);
    } finally {
      store.busy = null;
    }
  }

  function time(ts: string | null): string {
    if (!ts) return "";
    const d = new Date(ts);
    return Number.isNaN(d.getTime()) ? "" : d.toLocaleTimeString();
  }
</script>

<section class="controls">
  <div class="state">
    {#if store.autoRunning}<span class="dot"></span><strong>Running</strong>{:else}<strong>Off</strong>{/if}
    <span class="caption secondary">Switches to another account before the active one runs out.</span>
  </div>
  <div class="buttons">
    <button class:primary={!store.autoRunning} onclick={() => store.toggleAuto()}>
      {store.autoRunning ? "Stop" : "Start auto-switch"}
    </button>
    <button disabled={!!store.busy} onclick={runOnce}>Check once now</button>
  </div>
  <label class="check">
    <input
      type="checkbox"
      checked={store.prefs?.autoSwitchDryRun ?? false}
      onchange={(e) => store.savePrefs({ autoSwitchDryRun: e.currentTarget.checked })}
    />
    Dry run: report what it would do, switch nothing (applies on the next start)
  </label>
  <label class="check">
    <input
      type="checkbox"
      checked={store.prefs?.autoSwitchOnLaunch ?? false}
      onchange={(e) => store.savePrefs({ autoSwitchOnLaunch: e.currentTarget.checked })}
    />
    Start auto-switch when HaryX starts
  </label>
  <p class="caption tertiary">Thresholds and strategy are haryx settings: see Settings › haryx settings.</p>
</section>

<div class="hairline"></div>

<div class="log-head">
  <strong>Log</strong>
  <label class="check caption"><input type="checkbox" bind:checked={hidePolls} /> Hide routine checks</label>
</div>
{#if shown.length === 0}
  <p class="secondary">Nothing yet.</p>
{:else}
  <ol class="log">
    {#each shown as e, i (i)}
      <li class={e.severity}>
        <span class="tertiary time">{time(e.ts)}</span>
        <span>{e.summary}</span>
      </li>
    {/each}
  </ol>
{/if}

<style>
  .controls {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .state,
  .buttons,
  .log-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .log-head {
    justify-content: space-between;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .log {
    list-style: none;
    padding: 0;
    margin: 8px 0 0;
  }
  .log li {
    display: grid;
    grid-template-columns: 80px 1fr;
    gap: 8px;
    padding: 4px 0;
    border-top: 1px solid var(--separator);
  }
  .log li.problem {
    color: var(--red);
  }
  .log li.notable span:last-child {
    font-weight: 600;
  }
  p {
    margin: 0;
  }
</style>
