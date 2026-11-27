<script lang="ts">
  // Asks for a HaryX consent: what the user agrees to, the privacy policy version and a link to it.
  // Mirrors macOS ConsentSheet (HaryXAccountPane.swift).
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { CONSENT_COPY, POLICY_VERSION, PRIVACY_POLICY } from "../lib/consent";
  import { haryxAccount as acct } from "../lib/haryx.svelte";
  import { store } from "../lib/store.svelte";
  import type { ConsentPurpose } from "../lib/types";

  let { purpose, onclose }: { purpose: ConsentPurpose; onclose: () => void } = $props();

  let dialog: HTMLDialogElement;
  let error = $state<string | null>(null);
  let copy = $derived(CONSENT_COPY[purpose]);

  onMount(() => dialog.showModal());

  async function agree(e: SubmitEvent) {
    e.preventDefault();
    error = await acct.setConsent(purpose, true);
    if (!error) dialog.close();
  }
</script>

<dialog bind:this={dialog} aria-labelledby="consent-title" onclose={onclose}>
  <form onsubmit={agree}>
    <h2 id="consent-title">{copy.title}</h2>
    <ul>
      {#each copy.points as point (point)}<li>{point}</li>{/each}
    </ul>
    <button
      type="button"
      class="link"
      onclick={() => api.openLink(PRIVACY_POLICY).catch((e) => store.flash(String(e), true))}
    >
      Privacy policy, version {POLICY_VERSION}
    </button>
    {#if error}<p class="caption error-text" role="alert">{error}</p>{/if}
    <div class="buttons">
      {#if acct.working}<span class="spinner" aria-hidden="true"></span>{/if}
      <button type="button" onclick={() => dialog.close()}>Cancel</button>
      <button type="submit" class="primary" disabled={acct.working}>{copy.agree}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: min(460px, calc(100vw - 32px));
    padding: 20px;
    border: 1px solid var(--separator);
    border-radius: var(--radius-card);
    background: var(--surface);
    color: inherit;
  }
  dialog::backdrop {
    background: rgb(0 0 0 / 0.3);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h2 {
    margin: 0;
    font-size: 16px;
  }
  ul {
    margin: 0;
    padding-left: 18px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  p {
    margin: 0;
  }
  .link {
    align-self: flex-start;
  }
  .error-text {
    color: var(--red);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 8px;
  }
  .spinner {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid var(--track);
    border-top-color: var(--cyan);
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
