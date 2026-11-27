<script lang="ts">
  // The locked state of a synced section: a passphrase form. The passphrase goes to the unlock
  // worker and is dropped with the form; it is never stored.
  import State from "./State.svelte";
  import { syncLock } from "../lib/synclock";
  import type { SyncLock } from "../lib/unlock.svelte";

  let { lock = syncLock }: { lock?: SyncLock } = $props();
  let passphrase = $state("");
  const busy = $derived(lock.status === "unlocking");

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!passphrase || busy) return;
    const typed = passphrase;
    passphrase = "";
    await lock.unlock(typed);
  }
</script>

<State
  kind="locked"
  title="Locked: enter your sync passphrase"
  detail="Accounts, usage and profiles are end-to-end encrypted. Your passphrase is never stored."
>
  <form onsubmit={submit}>
    <label for="sync-passphrase">Sync passphrase</label>
    <div class="row">
      <input
        id="sync-passphrase"
        type="password"
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        bind:value={passphrase}
        disabled={busy}
        aria-invalid={lock.error?.code === "wrong_passphrase" ? "true" : undefined}
        aria-describedby={lock.error ? "unlock-error" : undefined}
      />
      <button class="primary" type="submit" disabled={busy || !passphrase}>{busy ? "Unlocking…" : "Unlock"}</button>
    </div>
    {#if busy}
      <div class="progress" role="status">
        <progress aria-label="Unlocking"></progress>
        <span>Unlocking in this tab. This takes a second or two.</span>
      </div>
    {:else if lock.error}
      <p id="unlock-error" class="error" role="alert">{lock.error.message}</p>
    {/if}
    <p class="note">Unlocking decrypts your data in this tab only. It locks again after 15 minutes idle or when you close the tab.</p>
  </form>
</State>

<style>
  form { display: grid; gap: .5rem; margin-top: .75rem; }
  label { font-weight: 600; font-size: .9rem; }
  .row { display: flex; gap: .5rem; flex-wrap: wrap; }
  input { flex: 1 1 12rem; font: inherit; color: inherit; background: var(--bg); border: 1px solid var(--separator); border-radius: var(--radius-control); padding: .4rem .6rem; min-height: 40px; min-width: 0; }
  input[aria-invalid="true"] { border-color: var(--red); }
  .progress { display: flex; gap: .6rem; align-items: center; color: var(--label-2); font-size: .9rem; }
  progress { width: 6rem; }
  .error { color: var(--red); margin: 0; }
  .note { color: var(--label-2); font-size: .85rem; margin: 0; }
</style>
