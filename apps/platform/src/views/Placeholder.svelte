<script lang="ts">
  // Task 19 (Accounts, Usage, Profiles) replaces the body of each view; the section chrome, the
  // state components and the sync lock stay.
  import Section from "../components/Section.svelte";
  import State from "../components/State.svelte";
  import Unlock from "../components/Unlock.svelte";
  import { syncLock } from "../lib/synclock";
  let { title, blurb, task }: { title: string; blurb: string; task: string } = $props();
</script>

<Section {title} {blurb}>
  {#if syncLock.status === "unlocked"}
    <State kind="empty" title="Unlocked" detail={`Filled in by plan ${task}.`} onAction={syncLock.lock} actionLabel="Lock now" />
  {:else}
    <Unlock />
  {/if}
</Section>
