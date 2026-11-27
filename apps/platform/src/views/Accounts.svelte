<script lang="ts">
  import Section from "../components/Section.svelte";
  import SyncGate from "../components/SyncGate.svelte";
  import AccountList from "../components/AccountList.svelte";
</script>

<Section title="Accounts" blurb="Your Claude accounts across machines.">
  <SyncGate>
    {#snippet children(synced, consents)}
      <AccountList {synced} {consents} />
    {/snippet}
  </SyncGate>
</Section>
