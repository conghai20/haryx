<script lang="ts">
  import Section from "../components/Section.svelte";
  import SyncGate from "../components/SyncGate.svelte";
  import UsageReport from "../components/UsageReport.svelte";
</script>

<Section title="Usage" blurb="Per account, per day, all machines.">
  <SyncGate>
    {#snippet children(synced)}
      <UsageReport {synced} />
    {/snippet}
  </SyncGate>
</Section>
