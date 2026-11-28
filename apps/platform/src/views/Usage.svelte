<script lang="ts">
  import SyncGate from "../components/SyncGate.svelte";
  import UsageReport from "../components/UsageReport.svelte";
</script>

<SyncGate what="Your usage">
  {#snippet children(synced)}
    <UsageReport {synced} />
  {/snippet}
</SyncGate>
