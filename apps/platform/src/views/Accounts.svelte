<script lang="ts">
  import SyncGate from "../components/SyncGate.svelte";
  import State from "../components/State.svelte";
  import AccountRows from "../components/AccountRows.svelte";
  import type { Overview } from "../lib/overview";
  import { isGranted } from "../lib/consents";
  import { vault } from "../lib/vault.svelte";

  let { view, onOpen }: { view: Overview | null; onOpen: (id: string) => void } = $props();
</script>

<SyncGate what="Your accounts">
  {#snippet children()}
    {#if !view || view.accounts.length === 0}
      <State kind="empty" title="No synced accounts yet" detail="Accounts appear here after a Mac or PC with Sync turned on runs a sync." />
    {:else}
      <AccountRows {view} {onOpen} />
      <p class="text-ink-3 mt-6 max-w-2xl text-sm">
        Readings are the newest your machines synced, not live.
        {#if vault.consents && !isGranted(vault.consents, "machines")}Machine names show when Machines is on in the <a href="#privacy" class="text-live-text">Privacy center</a>.{:else if vault.machinesFailed}Machine names did not load, so ids stand in.{/if}
        Sign-in tokens sync encrypted and are never shown here.
      </p>
    {/if}
  {/snippet}
</SyncGate>
