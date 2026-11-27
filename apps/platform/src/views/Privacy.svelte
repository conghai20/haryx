<script lang="ts">
  import Section from "../components/Section.svelte";
  import State from "../components/State.svelte";
  import { accountConsoleUrl } from "../lib/config";
</script>

<Section title="Privacy center" blurb="Your consents, your data export, and your WeCoAp account.">
  <State kind="empty" title="Consents: coming soon" detail="Grant, withdraw and history arrive with plan task 17." />
  <section class="account">
    <h2>Your WeCoAp account</h2>
    <p>Password, linked sign-in methods, sessions and deleting your account are managed by WeCoAp. Deleting it removes your WeCoAp ID for every WeCoAp product.</p>
    <a class="btn" href={accountConsoleUrl()} rel="noopener">Open WeCoAp account</a>
  </section>
</Section>

<style>
  .account { margin-top: 1.5rem; max-width: 36rem; }
</style>
