<script lang="ts">
  // AI settings: the app's AI-side preferences (usage refresh, tray, terminal, notifications), then
  // haryx's own settings (`haryx config`) in plain words, grouped as on macOS. The auto-switch rules
  // live in Auto-switch only. Every control saves as it changes.
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { actions } from "../lib/actions";
  import { api } from "../lib/api";
  import { config } from "../lib/config.svelte";
  import { openPane } from "../lib/nav";
  import { otherSections } from "../lib/settings";
  import { store } from "../lib/store.svelte";
  import type { Prefs, TerminalId } from "../lib/types";
  import ConfigRow from "../components/ConfigRow.svelte";

  /** The refresh choices macOS offers. */
  const REFRESH_CHOICES = [30, 60, 120, 300];
  /** The shells a Windows Terminal tab can run (the Rust side's TAB_SHELLS). */
  const TAB_SHELLS: TerminalId[] = ["pwsh", "powershell", "cmd"];

  // The macOS words for each notification (NotifyToggle).
  const notifications: { key: keyof Prefs; label: string }[] = [
    { key: "notifySwitched", label: "Account switched" },
    { key: "notifyUsage", label: "Usage reaches 80% / 95% or is on pace to run out" },
    { key: "notifyLogin", label: "Re-login needed, login expiring, plan expired" },
    { key: "notifyReset", label: "Exhausted account is ready again" },
    { key: "notifySession", label: "Claude session finished (ran 1 min or longer)" },
    { key: "notifyAutoSwitch", label: "Auto-switch problems (quarantine, all exhausted, errors)" },
  ];

  let sections = $derived(otherSections(config.entries ?? []));
  let shells = $derived(store.terminals.filter((t) => TAB_SHELLS.includes(t.id)));
  let refreshChoices = $derived(
    store.prefs && !REFRESH_CHOICES.includes(store.prefs.refreshSeconds)
      ? [...REFRESH_CHOICES, store.prefs.refreshSeconds].sort((a, b) => a - b)
      : REFRESH_CHOICES,
  );

  const refreshLabel = (s: number) => (s < 60 ? `${s} seconds` : s === 60 ? "1 minute" : `${s / 60} minutes`);

  async function chooseFolder() {
    try {
      const folder = await open({ directory: true, title: "Folder Claude starts in" });
      if (typeof folder === "string") await store.savePrefs({ claudeFolder: folder });
    } catch (e) {
      store.fail(e, "Couldn't choose a folder");
    }
  }

  async function testNotification() {
    try {
      await api.notifyTest();
    } catch (e) {
      store.fail(e, "Couldn't send the test notification");
    }
  }

  onMount(() => void config.load());
</script>

{#if store.prefs}
  {@const prefs = store.prefs}
  <section aria-labelledby="s-usage">
    <h2 id="s-usage">Usage & tray</h2>
    <div class="row">
      <label for="refresh">Refresh usage every</label>
      <select
        id="refresh"
        value={String(prefs.refreshSeconds)}
        onchange={(e) => store.savePrefs({ refreshSeconds: Number(e.currentTarget.value) })}
      >
        {#each refreshChoices as s (s)}<option value={String(s)}>{refreshLabel(s)}</option>{/each}
      </select>
    </div>
    <div class="row">
      <label for="tray-mascot">Show animated mascot in the tray</label>
      <input
        id="tray-mascot"
        type="checkbox"
        role="switch"
        class="switch"
        title="Replaces the HaryX mark with the active account's mascot, which walks while Claude runs."
        checked={prefs.trayMascot}
        onchange={(e) => store.savePrefs({ trayMascot: e.currentTarget.checked })}
      />
    </div>
  </section>

  <section aria-labelledby="s-terminal">
    <h2 id="s-terminal">Terminal</h2>
    <div class="row">
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
    <div class="row">
      <div class="words">
        <label for="shell">Shell</label>
        <span class="caption">For Windows Terminal tabs. The other terminals are their own shell.</span>
      </div>
      <select
        id="shell"
        value={prefs.shell ?? ""}
        disabled={prefs.terminal != null && prefs.terminal !== "windows-terminal"}
        onchange={(e) => store.savePrefs({ shell: (e.currentTarget.value || null) as TerminalId | null })}
      >
        <option value="">Automatic (PowerShell 7 first)</option>
        {#each shells as t (t.id)}<option value={t.id}>{t.title}</option>{/each}
      </select>
    </div>
    <div class="row">
      <span class="label">Default folder</span>
      <span class="folder">
        <span class="truncate secondary" title={prefs.claudeFolder ?? ""}>{prefs.claudeFolder ?? "Your home folder"}</span>
        <button onclick={chooseFolder}>Choose…</button>
        {#if prefs.claudeFolder}<button class="plain" onclick={() => store.savePrefs({ claudeFolder: null })}>Reset</button>{/if}
      </span>
    </div>
    <div><button onclick={() => actions.terminal({ kind: "shell" })}>Test: open a new shell</button></div>
    {#if prefs.terminal === "warp"}
      <p class="caption">
        Warp opens through a launch configuration in Warp's data folder and uses its own shell setting (the Shell choice
        above doesn't apply).
      </p>
    {/if}
  </section>

  <section aria-labelledby="s-notify">
    <h2 id="s-notify">Notifications</h2>
    {#each notifications as n (n.key)}
      <div class="row">
        <label for="n-{n.key}">{n.label}</label>
        <input
          id="n-{n.key}"
          type="checkbox"
          role="switch"
          class="switch"
          checked={Boolean(prefs[n.key])}
          onchange={(e) => store.savePrefs({ [n.key]: e.currentTarget.checked })}
        />
      </div>
    {/each}
    <div><button onclick={testNotification}>Send test notification</button></div>
  </section>
{/if}

{#if config.error}
  <div class="banner error" role="alert" title={config.errorDetail ?? ""}>
    <span>{config.error}</span>
    <button class="mini" onclick={() => config.load()}>Try again</button>
  </div>
{:else if !config.entries}
  <p class="secondary" role="status">Reading haryx's settings…</p>
{:else}
  {#each sections as s (s.section)}
    <section aria-labelledby="s-{s.section}">
      <h2 id="s-{s.section}">{s.title}</h2>
      <div class="rules">
        {#each s.entries as entry (entry.key)}<ConfigRow {entry} />{/each}
      </div>
    </section>
  {/each}
  <p class="caption">
    Auto-switch rules are in <button class="link" onclick={() => openPane("auto")}>Auto-switch</button>. Unknown keys and
    hand edits in <span title={config.path ?? ""}>haryx's settings file</span> are kept.
  </p>
{/if}

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-bottom: 20px;
  }
  h2 {
    font-size: 13px;
    font-weight: 600;
    margin: 0 0 6px;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-height: 36px;
    padding: 4px 0;
    border-top: 1px solid var(--separator);
  }
  .words {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .folder {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    max-width: 60%;
  }
  .rules {
    display: flex;
    flex-direction: column;
  }
  section > div:last-child:not(.row):not(.rules) {
    padding-top: 8px;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 12px;
  }
  .banner span {
    flex: 1;
  }
  .link {
    background: none;
    padding: 0;
    color: inherit;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  p {
    margin: 0;
  }
</style>
