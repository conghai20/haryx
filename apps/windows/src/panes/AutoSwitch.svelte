<script lang="ts">
  // Auto-switch: start or stop the `haryx auto --json` loop, run one check, its rules (the CLI's
  // autoswitch.* settings, here only), and the live log beside them, as macOS's split view.
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { config } from "../lib/config.svelte";
  import { ruleEntries } from "../lib/settings";
  import { store } from "../lib/store.svelte";
  import type { AutoEvent, RunResult } from "../lib/types";
  import ConfigRow from "../components/ConfigRow.svelte";
  import Icon, { type IconName } from "../components/Icon.svelte";

  let log = $derived([...store.autoLog].reverse());
  let hidePolls = $state(true);
  let shown = $derived(hidePolls ? log.filter((e) => e.kind !== "poll") : log);
  let rules = $derived(ruleEntries(config.entries ?? []));
  let lastPoll = $derived([...store.autoLog].reverse().find((e) => e.kind === "poll") ?? null);

  onMount(() => void config.load());

  async function runOnce() {
    store.busy = "Checking";
    try {
      const args = ["auto", "--once", "--json", ...(store.prefs?.autoSwitchDryRun ? ["--dry-run"] : [])];
      const r: RunResult = await api.haryx(args);
      if (r.status === 1) store.fail(r.message, "The check failed");
      else store.flash(r.status === 0 ? "Switched" : r.status === 2 ? "No switch needed" : "No account can take over right now");
    } catch (e) {
      store.fail(e, "The check failed");
    } finally {
      store.busy = null;
    }
  }

  function time(ts: string | null): string {
    if (!ts) return "";
    const d = new Date(ts);
    return Number.isNaN(d.getTime()) ? "" : d.toLocaleTimeString();
  }

  function ago(ts: string | null): string | null {
    if (!ts) return null;
    const minutes = Math.round((store.now - new Date(ts).getTime()) / 60_000);
    if (Number.isNaN(minutes)) return null;
    return minutes < 1 ? "just now" : minutes < 60 ? `${minutes} min ago` : `${Math.floor(minutes / 60)} h ago`;
  }

  const severityIcon: Record<AutoEvent["severity"], IconName> = {
    info: "refresh",
    notable: "switch",
    problem: "warning",
  };
</script>

<div class="split">
  <div class="main">
    <section class="controls" aria-labelledby="auto-state">
      <div class="state">
        <span class="mark" class:live={store.autoRunning}><Icon name="autoSwitch" size={24} /></span>
        <div class="state-text">
          <strong id="auto-state">{store.autoRunning ? "Auto-switch is running" : "Auto-switch is off"}</strong>
          {#if ago(lastPoll?.ts ?? null)}<span class="caption">Last check {ago(lastPoll?.ts ?? null)}</span>{/if}
        </div>
        <button class:primary={!store.autoRunning} onclick={() => store.toggleAuto()}>
          {store.autoRunning ? "Stop" : "Start"}
        </button>
      </div>
      <label class="line">
        <span>Dry run: report what it would do, switch nothing</span>
        <input
          type="checkbox"
          role="switch"
          class="switch"
          disabled={store.autoRunning}
          checked={store.prefs?.autoSwitchDryRun ?? false}
          onchange={(e) => store.savePrefs({ autoSwitchDryRun: e.currentTarget.checked })}
        />
      </label>
      <label class="line">
        <span>Start auto-switch when HaryX starts</span>
        <input
          type="checkbox"
          role="switch"
          class="switch"
          checked={store.prefs?.autoSwitchOnLaunch ?? false}
          onchange={(e) => store.savePrefs({ autoSwitchOnLaunch: e.currentTarget.checked })}
        />
      </label>
      <div><button disabled={!!store.busy} onclick={runOnce}>Check once now</button></div>
      <p class="caption">
        When the active account reaches your switch line, HaryX moves to another of your own accounts, and every decision
        appears in the log. Plan limits are set by Anthropic; use only accounts you hold yourself.
      </p>
    </section>

    <section aria-labelledby="rules-title">
      <h2 id="rules-title">Rules</h2>
      {#if config.error}
        <div class="banner error" role="alert" title={config.errorDetail ?? ""}>
          <span>{config.error}</span>
          <button class="mini" onclick={() => config.load()}>Try again</button>
        </div>
      {:else if !config.entries}
        <p class="secondary" role="status">Reading the rules…</p>
      {:else}
        <div class="rules">
          {#each rules as entry (entry.key)}<ConfigRow {entry} />{/each}
        </div>
      {/if}
    </section>
  </div>

  <aside class="log-pane" aria-labelledby="log-title">
    <div class="log-head">
      <h2 id="log-title">Log</h2>
      <label class="check caption"><input type="checkbox" bind:checked={hidePolls} /> Hide routine checks</label>
    </div>
    {#if shown.length === 0}
      <p class="secondary empty">No events yet.</p>
    {:else}
      <ol class="log">
        {#each shown as e, i (`${e.ts ?? ""}-${e.kind}-${i}`)}
          <li class={e.severity}>
            <span class="ev-icon"><Icon name={severityIcon[e.severity] ?? "refresh"} size={14} /></span>
            <span class="ev-text">
              <span>{e.summary}</span>
              <span class="caption tertiary">{time(e.ts)}</span>
            </span>
          </li>
        {/each}
      </ol>
    {/if}
  </aside>
</div>

<style>
  .split {
    display: grid;
    grid-template-columns: minmax(360px, 720px) minmax(260px, 1fr);
    gap: 24px;
    align-items: start;
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .controls {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .state {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .state-text {
    flex: 1;
    display: flex;
    flex-direction: column;
  }
  .mark {
    color: var(--label-2);
  }
  .mark.live {
    color: var(--cyan-text);
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  h2 {
    font-size: 13px;
    font-weight: 600;
    margin: 0 0 4px;
  }
  .rules {
    display: flex;
    flex-direction: column;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .banner span {
    flex: 1;
  }
  .log-pane {
    position: sticky;
    top: 0;
    padding-left: 20px;
    border-left: 1px solid var(--separator);
  }
  .log-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .log {
    list-style: none;
    padding: 0;
    margin: 8px 0 0;
  }
  .log li {
    display: flex;
    gap: 8px;
    padding: 6px 0;
    border-top: 1px solid var(--separator);
  }
  .ev-icon {
    color: var(--label-3);
    padding-top: 2px;
  }
  .ev-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .log li.notable .ev-icon {
    color: var(--cyan-text);
  }
  .log li.notable .ev-text > span:first-child {
    font-weight: 600;
  }
  .log li.problem,
  .log li.problem .ev-icon {
    color: var(--red);
  }
  .empty {
    margin-top: 24px;
    text-align: center;
  }
  p {
    margin: 0;
  }
  @media (max-width: 900px) {
    .split {
      grid-template-columns: 1fr;
    }
    .log-pane {
      position: static;
      padding-left: 0;
      border-left: none;
    }
  }
</style>
