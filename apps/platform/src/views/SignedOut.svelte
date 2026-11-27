<script lang="ts">
  import Lockup from "@win/components/Lockup.svelte";
  let { onSignIn }: { onSignIn: () => void } = $props();
</script>

<div class="first">
  <Lockup size={44} />
  <p>Your machines, accounts and privacy choices for HaryX, in the browser.</p>
  <button class="primary" onclick={onSignIn}>Sign in with WeCoAp</button>
</div>

<style>
  .first { min-height: 70vh; display: grid; place-content: center; justify-items: center; gap: 1rem; text-align: center; padding: 0 1rem; }
  p { color: var(--label-2); max-width: 24rem; margin: 0; }
</style>
