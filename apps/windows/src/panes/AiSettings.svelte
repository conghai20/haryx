<script lang="ts">
  // AI settings: the app's AI-side preferences (usage refresh, tray, terminal, notifications) above
  // haryx's own settings (`config list/set/unset`). The app's own preferences are in Preferences.
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "../lib/api";
  import { store } from "../lib/store.svelte";
  import type { Prefs, TerminalId } from "../lib/types";

  interface ConfigEntry {
    key: string;
    value: string | number | boolean | null;
    isSet: boolean;
  }

  let config = $state<{ path: string; settings: ConfigEntry[] } | null>(null);
  let edits = $state<Record<string, string>>({});
  let configError = $state<string | null>(null);

  const notifications: { key: keyof Prefs; label: string }[] = [
    { key: "notifySwitched", label: "Account switched" },
    { key: "notifyUsage", label: "Usage at 80% and 95%, and running out early" },
    { key: "notifyLogin", label: "Sign-in needed or expiring, plan expired" },
    { key: "notifyReset", label: "Quota reset after running out" },
    { key: "notifySession", label: "Claude session finished (ran 1 min or longer)" },
    { key: "notifyAutoSwitch", label: "Auto-switch problems" },
  ];

  async function loadConfig() {
    try {
      config = await api.json<{ path: string; settings: ConfigEntry[] }>(["config", "list", "--json"]);
      edits = Object.fromEntries(config.settings.map((s) => [s.key, s.value == null ? "" : String(s.value)]));
      configError = null;
    } catch (e) {
      configError = String(e);
    }
  }

  async function setKey(key: string) {
    const value = (edits[key] ?? "").trim();
    if (value === "") return;
    if (await store.run(`Setting ${key}`, ["config", "set", key, value])) await loadConfig();
  }

  async function unsetKey(key: string) {
    if (await store.run(`Resetting ${key}`, ["config", "unset", key])) await loadConfig();
  }

  async function chooseFolder() {
    const folder = await open({ directory: true, title: "Folder Claude starts in" });
    if (typeof folder === "string") await store.savePrefs({ claudeFolder: folder });
  }


  onMount(loadConfig);
</script>

{#if store.prefs}
  {@const prefs = store.prefs}
  <section>
    <h2>Usage & tray</h2>
    <div class="field">
      <label for="refresh">Refresh accounts every</label>
      <select id="refresh" value={String(prefs.refreshSeconds)} onchange={(e) => store.savePrefs({ refreshSeconds: Number(e.currentTarget.value) })}>
        {#each [30, 60, 120, 300, 600] as s (s)}<option value={String(s)}>{s < 60 ? `${s} seconds` : `${s / 60} min`}</option>{/each}
      </select>
    </div>
    <label class="check">
      <input type="checkbox" checked={prefs.trayMascot} onchange={(e) => store.savePrefs({ trayMascot: e.currentTarget.checked })} />
      Show the active account's mascot in the tray (it walks while Claude runs)
    </label>
  </section>

  <div class="hairline"></div>

  <section>
    <h2>Terminal</h2>
    <div class="field">
      <label for="terminal">Open terminals in</label>
      <select
        id="terminal"
        value={prefs.terminal ?? ""}
        onchange={(e) => store.savePrefs({ terminal: (e.currentTarget.value || null) as TerminalId | null })}
      >
        <option value="">First available</option>
        {#each store.terminals as t (t.id)}<option value={t.id}>{t.title}</option>{/each}
      </select>
    </div>
    {#if prefs.terminal === "warp"}
      <p class="caption tertiary">
        Warp opens through a launch configuration in Warp's data folder and runs in Warp's own shell (PowerShell unless
        you changed it in Warp).
      </p>
    {/if}
    <div class="field">
      <span class="label">Claude starts in</span>
      <span class="truncate value" title={prefs.claudeFolder ?? ""}>{prefs.claudeFolder ?? "your home folder"}</span>
      <button onclick={chooseFolder}>Choose…</button>
      {#if prefs.claudeFolder}<button class="plain" onclick={() => store.savePrefs({ claudeFolder: null })}>Reset</button>{/if}
    </div>
  </section>

  <div class="hairline"></div>

  <section>
    <h2>Notifications</h2>
    {#each notifications as n (n.key)}
      <label class="check">
        <input type="checkbox" checked={Boolean(prefs[n.key])} onchange={(e) => store.savePrefs({ [n.key]: e.currentTarget.checked })} />
        {n.label}
      </label>
    {/each}
  </section>
{/if}

<div class="hairline"></div>

<section>
  <h2>haryx settings</h2>
  {#if configError}
    <div class="banner error">{configError}</div>
  {:else if !config}
    <p class="secondary">Loading…</p>
  {:else}
    <p class="caption tertiary">{config.path}</p>
    <div class="config">
      {#each config.settings as s (s.key)}
        <form class="config-row" onsubmit={(e) => { e.preventDefault(); setKey(s.key); }}>
          <label for="cfg-{s.key}" class:set={s.isSet}>{s.key}</label>
          <input id="cfg-{s.key}" bind:value={edits[s.key]} />
          <button type="submit">Save</button>
          {#if s.isSet}<button type="button" class="plain" onclick={() => unsetKey(s.key)}>Reset</button>{:else}<span></span>{/if}
        </form>
      {/each}
    </div>
  {/if}
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h2 {
    font-size: 13px;
    font-weight: 600;
    margin: 0 0 4px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .field {
    display: grid;
    grid-template-columns: 170px minmax(0, 1fr) auto auto;
    align-items: center;
    gap: 8px;
  }
  .field select {
    justify-self: start;
  }
  .label,
  .field label {
    color: var(--label-2);
  }
  .value {
    color: var(--label-2);
  }
  .config {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .config-row {
    display: grid;
    grid-template-columns: 240px minmax(0, 1fr) auto 60px;
    align-items: center;
    gap: 8px;
  }
  .config-row label {
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
    color: var(--label-2);
  }
  .config-row label.set {
    color: var(--label);
    font-weight: 600;
  }
  p {
    margin: 0;
  }
</style>
