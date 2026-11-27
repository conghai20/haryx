<script lang="ts">
  // HaryX › About & updates: versions, how updates install, and the latest release's notes.
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { blocks, codeSpans } from "../lib/notes";
  import { store } from "../lib/store.svelte";
  import type { UpdateMode } from "../lib/types";
  import Lockup from "../components/Lockup.svelte";

  const modes: { id: UpdateMode; label: string }[] = [
    { id: "automatic", label: "Install automatically" },
    { id: "notify", label: "Notify me" },
    { id: "manual", label: "Only when I check" },
  ];

  let info = $derived(store.update);
  let phase = $derived(info?.state.phase ?? "idle");
  let check = $derived(info?.state.check ?? null);
  let notes = $derived(blocks(check?.notes ?? ""));
  let lastCheck = $derived(info?.lastCheck ? new Date(info.lastCheck * 1000).toLocaleString() : null);

  async function call(f: () => Promise<void>) {
    try {
      await f();
    } catch (e) {
      store.fail(e, "The update step failed");
    }
  }

  onMount(() => {
    if (!check) call(() => api.updateCheck(false));
  });
</script>

{#if info}
  <section>
    <div class="about">
      <Lockup size={24} />
      <span class="secondary">Version {info.state.appVersion}</span>
    </div>
    <p class="caption tertiary">haryx: {store.cliPath ?? "not found"}</p>
    <p class="links">
      <button class="link" onclick={() => call(() => api.openLink("https://github.com/conghai20/haryx"))}>GitHub</button>
      <button class="link" onclick={() => call(() => api.openLink("https://github.com/conghai20/haryx/releases"))}>All releases</button>
      <button class="link" onclick={() => call(() => api.openLink("https://github.com/conghai20/haryx/blob/main/LICENSE"))}>Licence (MIT)</button>
    </p>
  </section>

  <div class="hairline"></div>

  <section>
    <h2>Updates</h2>
    <div class="field">
      <label for="update-mode">Updates</label>
      <select
        id="update-mode"
        value={info.mode}
        onchange={(e) => call(() => api.updateSet({ mode: e.currentTarget.value as UpdateMode }))}
      >
        {#each modes as m (m.id)}<option value={m.id}>{m.label}</option>{/each}
      </select>
    </div>

    <div class="status" role="status">
      {#if phase === "checking"}
        <span class="secondary">Checking…</span>
      {:else if phase === "downloading"}
        <span class="secondary">Downloading {info.available ?? ""}…</span>
      {:else if phase === "ready"}
        <strong>HaryX {info.state.readyVersion} is ready.</strong>
        <span class="caption secondary">It installs when you quit HaryX, or now:</span>
        <button class="primary" onclick={() => call(() => api.updateRestart())}>Restart now</button>
      {:else if phase === "failed"}
        <span class="error-text">{info.state.error}</span>
      {:else if info.available}
        <strong>HaryX {info.available} is available.</strong>
        <button onclick={() => call(() => api.updateSet({ skip: info.available ?? "" }))}>Skip this version</button>
        <button class="primary" onclick={() => call(() => api.updateDownload())}>Update</button>
      {:else if info.skipped && check?.latest === info.skipped}
        <span class="secondary">You skipped {info.skipped}.</span>
        <button class="plain" onclick={() => call(() => api.updateSet({ skip: "" }))}>Show it again</button>
      {:else if check?.latest}
        <span class="secondary">HaryX is up to date.</span>
      {/if}
    </div>

    <div class="row">
      <button disabled={phase === "checking" || phase === "downloading"} onclick={() => call(() => api.updateCheck(true))}>
        Check now
      </button>
      {#if lastCheck}<span class="caption tertiary">Last checked {lastCheck}</span>{/if}
    </div>
    <p class="caption tertiary">
      Updates come from GitHub releases. Each installer is checked against its SHA-256 before it runs;
      the bundled haryx updates with the app.
    </p>
  </section>

  {#if notes.length > 0}
    <div class="hairline"></div>
    <section class="notes">
      <h2>{info.available ? `What's new in ${check?.latest}` : `Release notes for ${check?.latest}`}</h2>
      {#each notes as block, i (i)}
        {#if block.kind === "heading"}
          <h3>{block.text}</h3>
        {:else}
          <p class:bullet={block.kind === "bullet"}>
            {#each codeSpans(block.text) as part, j (j)}{#if j % 2}<code>{part}</code>{:else}{part}{/if}{/each}
          </p>
        {/if}
      {/each}
      {#if check?.html_url}<button class="link" onclick={() => call(() => api.openLink(check?.html_url ?? ""))}>Open on GitHub</button>{/if}
    </section>
  {/if}
{/if}

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 0;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  h3 {
    margin: 8px 0 0;
    font-size: 13px;
    font-weight: 600;
  }
  .about {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .links {
    display: flex;
    gap: 16px;
    margin: 0;
  }
  .field,
  .row,
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .field label {
    min-width: 120px;
  }
  .error-text {
    color: var(--red, #c42b1c);
  }
  .notes p {
    margin: 0;
    max-width: 72ch;
  }
  .notes p.bullet {
    padding-left: 14px;
    text-indent: -10px;
  }
  .notes p.bullet::before {
    content: "• ";
    color: var(--label-2);
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent, #0067c0);
    cursor: pointer;
    text-decoration: underline;
    width: fit-content;
  }
  code {
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
    font-size: 12px;
  }
</style>
