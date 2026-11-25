<script lang="ts">
  // The app's own preferences: starting with Windows, its look, and which haryx it runs.
  // Everything about AI accounts is in AI settings.
  import { open } from "@tauri-apps/plugin-dialog";
  import { store } from "../lib/store.svelte";
  import type { Theme } from "../lib/types";

  const themes: { value: Theme; label: string }[] = [
    { value: "system", label: "System" },
    { value: "light", label: "Light" },
    { value: "dark", label: "Dark" },
  ];

  async function chooseHaryx() {
    const file = await open({ title: "haryx.exe", filters: [{ name: "haryx", extensions: ["exe"] }] });
    if (typeof file === "string") await store.savePrefs({ haryxPath: file });
  }
</script>

{#if store.prefs}
  {@const prefs = store.prefs}
  <section>
    <h2>General</h2>
    <label class="check">
      <input type="checkbox" checked={prefs.launchAtLogin} onchange={(e) => store.savePrefs({ launchAtLogin: e.currentTarget.checked })} />
      Start HaryX when you sign in to Windows
    </label>
    <div class="field">
      <label for="theme">Theme</label>
      <select id="theme" value={prefs.theme} onchange={(e) => store.savePrefs({ theme: e.currentTarget.value as Theme })}>
        {#each themes as t (t.value)}<option value={t.value}>{t.label}</option>{/each}
      </select>
    </div>
  </section>

  <div class="hairline"></div>

  <section>
    <h2>Command-line backend</h2>
    <div class="field">
      <span class="label">haryx</span>
      <span class="truncate value" title={store.cliPath ?? ""}>{store.cliPath ?? "not found"}</span>
      <button onclick={chooseHaryx}>Choose…</button>
      {#if prefs.haryxPath}<button class="plain" onclick={() => store.savePrefs({ haryxPath: null })}>Use PATH</button>{/if}
    </div>
    {#if store.storeRoot}<p class="caption tertiary">Data folder: {store.storeRoot}</p>{/if}
  </section>
{/if}

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
  .field label,
  .value {
    color: var(--label-2);
  }
  p {
    margin: 0;
  }
</style>
