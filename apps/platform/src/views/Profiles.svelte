<script lang="ts">
  import SyncGate from "../components/SyncGate.svelte";
  import ProfileList from "../components/ProfileList.svelte";
</script>

<SyncGate what="Your profiles">
  {#snippet children(synced)}
    <ProfileList {synced} />
  {/snippet}
</SyncGate>
