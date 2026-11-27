<script lang="ts" module>
  import type { SyncRow, SyncKey } from "../lib/rowcrypto";
  import { decodeSynced, type Synced } from "../lib/synced";

  // One decode per unlock, shared by the three sections. Keyed by the rows array the lock holds, so
  // a lock (which replaces it) lets the decoded data go with it.
  const decoded = new WeakMap<readonly SyncRow[], Promise<Synced>>();
  function decode(key: SyncKey, rows: readonly SyncRow[]): Promise<Synced> {
    let p = decoded.get(rows);
    if (!p) {
      p = decodeSynced(key, rows);
      decoded.set(rows, p);
    }
    return p;
  }
</script>

<script lang="ts">
  // The states every synced section shares, in order: consents loading or failed, Sync consent
  // missing, locked (the passphrase form), decrypting, then the section itself. Signed out is the
  // shell's: a 401 anywhere drops the whole app to the sign-in screen.
  import { onMount, type Snippet } from "svelte";
  import State from "./State.svelte";
  import Unlock from "./Unlock.svelte";
  import { api } from "../lib/auth.svelte";
  import { ApiError } from "../lib/api";
  import { isGranted } from "../lib/consents";
  import { syncLock } from "../lib/synclock";
  import type { Consent } from "../lib/types";

  let { children }: { children: Snippet<[Synced, Consent[]]> } = $props();

  let consents = $state<Consent[] | null>(null);
  let consentsFailed = $state(false);
  let synced = $state.raw<Synced | null>(null);
  let decodeFailed = $state(false);

  async function loadConsents() {
    consentsFailed = false;
    consents = null;
    try {
      consents = (await api<{ consents: Consent[] }>("/consents")).consents;
    } catch (e) {
      if (!(e instanceof ApiError && e.kind === "unauthorized")) consentsFailed = true;
    }
  }
  onMount(loadConsents);

  $effect(() => {
    const rows = syncLock.rows;
    const key = syncLock.key;
    synced = null;
    decodeFailed = false;
    if (syncLock.status !== "unlocked" || !key) return;
    let live = true;
    decode(key, rows).then(
      (s) => { if (live) synced = s; },
      () => { if (live) decodeFailed = true; },
    );
    return () => { live = false; };
  });

  const openPrivacy = () => (location.hash = "#privacy");
</script>

{#if consents === null && !consentsFailed}
  <State kind="loading" />
{:else if consentsFailed}
  <State kind="error" title="Could not check your consents" onAction={loadConsents} />
{:else if !isGranted(consents!, "sync") || syncLock.error?.code === "consent_required"}
  <State kind="consentMissing" title="Sync is off" detail="Accounts, usage and profiles come from Sync. Allow Sync in the Privacy center, then turn it on in the HaryX app." onAction={openPrivacy} />
{:else if syncLock.status !== "unlocked"}
  <Unlock />
{:else if decodeFailed}
  <State kind="error" title="Could not open your synced data" detail="Lock and unlock again to retry." onAction={syncLock.lock} actionLabel="Lock" />
{:else if synced === null}
  <State kind="loading" />
{:else}
  <div class="bar" role="status">
    <span>Unlocked in this tab. Locks after 15 minutes idle.</span>
    <button onclick={syncLock.lock}>Lock now</button>
  </div>
  {@render children(synced, consents!)}
{/if}

<style>
  .bar { display: flex; flex-wrap: wrap; gap: .5rem 1rem; align-items: center; justify-content: space-between; max-width: 52rem; margin-bottom: 1rem; color: var(--label-2); font-size: .85rem; }
</style>
