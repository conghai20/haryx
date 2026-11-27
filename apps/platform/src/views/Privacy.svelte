<script lang="ts">
  import { onMount } from "svelte";
  import Section from "../components/Section.svelte";
  import State from "../components/State.svelte";
  import Dialog from "../components/Dialog.svelte";
  import { api } from "../lib/auth.svelte";
  import { ApiError } from "../lib/api";
  import { accountConsoleUrl } from "../lib/config";
  import { consentRequest, PURPOSES, withdrawWarning } from "../lib/consents";
  import { exactTime } from "../lib/machines";
  import type { Consent, ConsentPurpose } from "../lib/types";

  let consents = $state<Consent[] | null>(null);
  let failed = $state(false);
  let withdrawing = $state<ConsentPurpose | null>(null);
  let busy = $state(false);
  let actionError = $state("");
  let exportError = $state("");

  async function load() {
    failed = false;
    consents = null;
    try {
      consents = (await api<{ consents: Consent[] }>("/consents")).consents;
    } catch (e) {
      if (!(e instanceof ApiError && e.kind === "unauthorized")) failed = true;
    }
  }
  onMount(load);

  async function change(purpose: ConsentPurpose, grant: boolean) {
    busy = true;
    actionError = "";
    const { path, ...init } = consentRequest(purpose, grant);
    try {
      consents = (await api<{ consents: Consent[] }>(path, init)).consents;
      withdrawing = null;
    } catch {
      actionError = "Could not save that change. Nothing was changed. Try again.";
    } finally {
      busy = false;
    }
  }

  async function download() {
    exportError = "";
    try {
      const data = await api<unknown>("/me/export");
      const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: "application/json" }));
      const a = Object.assign(document.createElement("a"), { href: url, download: `haryx-export-${new Date().toISOString().slice(0, 10)}.json` });
      a.click();
      URL.revokeObjectURL(url);
    } catch {
      exportError = "Could not prepare your export. Try again.";
    }
  }
</script>

<Section title="Privacy center" blurb="Your consents, your data export, and your WeCoAp account.">
  {#if consents === null && !failed}
    <State kind="loading" />
  {:else if failed}
    <State kind="error" title="Could not load your consents" onAction={load} />
  {:else}
    {#if actionError && !withdrawing}<p class="err" role="alert">{actionError}</p>{/if}
    {#each consents! as c (c.purpose)}
      <section class="card">
        <header>
          <h2>{PURPOSES[c.purpose].title}</h2>
          <span class="tag" class:on={c.granted}>{c.granted ? "On" : "Off"}</span>
        </header>
        <p>{PURPOSES[c.purpose].grants}</p>
        {#if c.at}<p class="meta">Version {c.version} · {c.granted ? "granted" : "withdrawn"} <time datetime={c.at}>{exactTime(c.at)}</time></p>{/if}
        {#if c.granted}
          <button class="danger" onclick={() => { actionError = ""; withdrawing = c.purpose; }}>Withdraw</button>
        {:else}
          <button class="primary" onclick={() => change(c.purpose, true)} disabled={busy}>Grant</button>
        {/if}
        {#if c.history.length}
          <details>
            <summary>History ({c.history.length})</summary>
            <ol>
              {#each [...c.history].reverse() as e}
                <li>{e.action === "grant" ? "Granted" : "Withdrawn"} · v{e.version} · {e.source} · <time datetime={e.at}>{exactTime(e.at)}</time></li>
              {/each}
            </ol>
          </details>
        {/if}
      </section>
    {/each}
  {/if}

  <section class="card">
    <h2>Your data</h2>
    <p>Download everything HaryX holds about you as one JSON file. HaryX data is erased after 12 months without use.</p>
    <button onclick={download}>Download my data</button>
    {#if exportError}<p class="err" role="alert">{exportError}</p>{/if}
  </section>

  <section class="card">
    <h2>Your WeCoAp account</h2>
    <p>Password, linked sign-in methods, sessions and deleting your account are managed by WeCoAp. Deleting it removes your WeCoAp ID for every WeCoAp product.</p>
    <a class="btn" href={accountConsoleUrl()} rel="noopener">Open WeCoAp account</a>
  </section>
</Section>

{#if withdrawing}
  <Dialog title={`Withdraw ${PURPOSES[withdrawing].title}?`} onClose={() => (withdrawing = null)}>
    <p>{withdrawWarning(withdrawing)}</p>
    {#if actionError}<p class="err" role="alert">{actionError}</p>{/if}
    <div class="row">
      <button onclick={() => (withdrawing = null)} disabled={busy}>Cancel</button>
      <button class="primary" onclick={() => change(withdrawing!, false)} disabled={busy}>Withdraw and erase</button>
    </div>
  </Dialog>
{/if}

<style>
  .card { background: var(--surface); border: 1px solid var(--separator); border-radius: var(--radius-card); padding: 1rem 1.25rem; max-width: 44rem; margin-bottom: .75rem; }
  header { display: flex; align-items: center; gap: .6rem; }
  .tag { font-size: .75rem; background: var(--fill-strong); border-radius: 99px; padding: .05rem .5rem; margin-bottom: .5rem; }
  .tag.on { background: var(--selected); }
  .meta { color: var(--label-2); font-size: .85rem; }
  .err { color: var(--red); font-size: .85rem; }
  details { margin-top: .75rem; font-size: .85rem; color: var(--label-2); }
  summary { cursor: pointer; }
  ol { margin: .4rem 0 0; padding-left: 1.2rem; }
  .row { display: flex; gap: .5rem; justify-content: flex-end; margin-top: 1rem; flex-wrap: wrap; }
</style>
