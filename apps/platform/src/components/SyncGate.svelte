<script lang="ts">
  // What a synced section shows until its data is open. The way in (passphrase, consent) lives in
  // the hero; this side only says why the section is empty and where to look.
  import type { Snippet } from "svelte";
  import State from "./State.svelte";
  import { vault } from "../lib/vault.svelte";
  import { syncLock } from "../lib/synclock";
  import type { Synced } from "../lib/synced";

  let { what, children }: { what: string; children: Snippet<[Synced]> } = $props();

  function focusPassphrase() {
    const input = document.getElementById("sync-passphrase");
    input?.scrollIntoView({ block: "center", behavior: "smooth" });
    input?.focus({ preventScroll: true });
  }
</script>

{#if vault.state === "ready" && vault.synced}
  {@render children(vault.synced)}
{:else if vault.state === "loading" || vault.state === "decoding"}
  <State kind="loading" />
{:else if vault.state === "error"}
  <State kind="error" title="Could not check your consents" onAction={() => vault.loadConsents()} />
{:else if vault.state === "syncOff"}
  <State kind="consentMissing" title="Sync is off" detail="{what} come from Sync. Allow Sync in the Privacy center, then turn it on in the HaryX app." onAction={() => (location.hash = "#privacy")} />
{:else if vault.state === "decodeFailed"}
  <State kind="error" title="Could not open your synced data" detail="Lock and unlock again to retry." onAction={() => syncLock.lock()} actionLabel="Lock" />
{:else}
  <State kind="locked" title="Locked" detail="{what} are end-to-end encrypted. Unlock with your sync passphrase to read them in this tab." onAction={focusPassphrase} actionLabel="Enter passphrase" />
{/if}
