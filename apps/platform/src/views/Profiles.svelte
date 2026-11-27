<script lang="ts">
  import Section from "../components/Section.svelte";
  import SyncGate from "../components/SyncGate.svelte";
  import ProfileList from "../components/ProfileList.svelte";
</script>

<Section title="Profiles" blurb="Synced profile files.">
  <SyncGate>
    {#snippet children(synced)}
      <ProfileList {synced} />
    {/snippet}
  </SyncGate>
</Section>
