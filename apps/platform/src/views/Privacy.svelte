<script lang="ts">
  import { onMount } from "svelte";
  import State from "../components/State.svelte";
  import * as AlertDialog from "$lib/components/ui/alert-dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { vault } from "../lib/vault.svelte";
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
      vault.setConsents(consents);
      if (purpose === "machines") void vault.loadMachines();
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

<p class="text-ink-2 mb-10 max-w-2xl">What HaryX may keep for you, your data export, and your WeCoAp account. Each choice is yours and can be changed here at any time.</p>

{#if consents === null && !failed}
  <State kind="loading" />
{:else if failed}
  <State kind="error" title="Could not load your consents" onAction={load} />
{:else}
  {#if actionError && !withdrawing}<p class="text-red-text mb-4 text-sm" role="alert">{actionError}</p>{/if}
  <div class="max-w-3xl">
    {#each consents! as c (c.purpose)}
      <section class="border-rule grid gap-3 border-t py-8 sm:grid-cols-[1fr_auto] sm:gap-x-10" aria-labelledby="consent-{c.purpose}">
        <div class="grid gap-2">
          <h3 id="consent-{c.purpose}" class="flex items-center gap-3 text-lg font-semibold">
            {PURPOSES[c.purpose].title}
            <span class="text-sm font-medium {c.granted ? 'text-live-text' : 'text-ink-3'}">{c.granted ? "On" : "Off"}</span>
          </h3>
          <p class="text-ink-2 max-w-xl">{PURPOSES[c.purpose].grants}</p>
          {#if c.at}<p class="text-ink-3 text-sm">{c.granted ? "Allowed" : "Withdrawn"} <time datetime={c.at}>{exactTime(c.at)}</time> · policy of {c.version}</p>{/if}
          {#if c.history.length}
            <details class="text-ink-3 mt-1 text-sm">
              <summary class="hover:text-ink cursor-pointer">History ({c.history.length})</summary>
              <ol class="mt-2 grid gap-1 pl-4">
                {#each [...c.history].reverse() as e}
                  <li>{e.action === "grant" ? "Allowed" : "Withdrawn"} · policy of {e.version} · from {e.source} · <time datetime={e.at}>{exactTime(e.at)}</time></li>
                {/each}
              </ol>
            </details>
          {/if}
        </div>
        <div class="sm:pt-1">
          {#if c.granted}
            <Button variant="destructive-outline" onclick={() => { actionError = ""; withdrawing = c.purpose; }}>Withdraw</Button>
          {:else}
            <Button onclick={() => change(c.purpose, true)} disabled={busy}>Allow</Button>
          {/if}
        </div>
      </section>
    {/each}
  </div>
{/if}

<section class="border-rule grid max-w-3xl gap-3 border-t py-8 sm:grid-cols-[1fr_auto] sm:gap-x-10" aria-labelledby="your-data">
  <div class="grid gap-2">
    <h3 id="your-data" class="text-lg font-semibold">Your data</h3>
    <p class="text-ink-2 max-w-xl">Download everything HaryX holds about you as one JSON file. HaryX erases its data after 12 months without use.</p>
    {#if exportError}<p class="text-red-text text-sm" role="alert">{exportError}</p>{/if}
  </div>
  <div class="sm:pt-1"><Button variant="outline" onclick={download}>Download my data</Button></div>
</section>

<section class="border-rule grid max-w-3xl gap-3 border-t border-b py-8 sm:grid-cols-[1fr_auto] sm:gap-x-10" aria-labelledby="wecoap-account">
  <div class="grid gap-2">
    <h3 id="wecoap-account" class="text-lg font-semibold">Your WeCoAp account</h3>
    <p class="text-ink-2 max-w-xl">Password, sign-in methods, sessions and deleting your account are managed by WeCoAp. Deleting it removes your WeCoAp ID for every WeCoAp product.</p>
  </div>
  <div class="sm:pt-1"><Button variant="outline" href={accountConsoleUrl()} target="_blank" rel="noopener">Open WeCoAp account</Button></div>
</section>

<AlertDialog.Root open={withdrawing !== null} onOpenChange={(open) => { if (!open && !busy) withdrawing = null; }}>
  <AlertDialog.Content>
    {#if withdrawing}
      <AlertDialog.Header>
        <AlertDialog.Title>Withdraw {PURPOSES[withdrawing].title}?</AlertDialog.Title>
        <AlertDialog.Description>{withdrawWarning(withdrawing)}</AlertDialog.Description>
      </AlertDialog.Header>
      <p class="text-ink-2 text-sm">Want a copy first? <button class="text-live-text underline underline-offset-3" onclick={download}>Download my data</button> before you withdraw.</p>
      {#if exportError}<p class="text-red-text text-sm" role="alert">{exportError}</p>{/if}
      {#if actionError}<p class="text-red-text text-sm" role="alert">{actionError}</p>{/if}
      <AlertDialog.Footer>
        <AlertDialog.Cancel disabled={busy}>Keep it on</AlertDialog.Cancel>
        <Button variant="destructive" onclick={() => change(withdrawing!, false)} disabled={busy}>{busy ? "Withdrawing…" : "Withdraw and erase"}</Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
