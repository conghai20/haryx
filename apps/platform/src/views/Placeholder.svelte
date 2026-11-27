<script lang="ts">
  // Tasks 17 (Machines, Privacy) and 19 (Accounts, Usage, Profiles) replace the body of each view;
  // the section chrome and the state components stay.
  import Section from "../components/Section.svelte";
  import State from "../components/State.svelte";
  let { title, blurb, task }: { title: string; blurb: string; task: string } = $props();
</script>

<Section {title} {blurb}>
  <State kind="empty" title="Coming soon" detail={`Filled in by plan ${task}.`} />
</Section>
