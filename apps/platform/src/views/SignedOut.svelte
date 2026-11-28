<script lang="ts">
  // The way in: the homepage's hero, with sign-in as its one action. WeCoAp is named and explained,
  // and the encryption promise is said before anything is asked.
  import { Button } from "$lib/components/ui/button/index.js";
  import Mark from "../components/brand/Mark.svelte";
  import Wordmark from "../components/brand/Wordmark.svelte";
  import TideLine from "../components/brand/TideLine.svelte";

  let { failed = false, onSignIn, onRetry }: { failed?: boolean; onSignIn: () => void; onRetry: () => void } = $props();
</script>

<div class="relative isolate flex min-h-dvh flex-col overflow-hidden">
  <header class="flex items-center justify-between px-4 pt-6 sm:px-8 lg:px-14 lg:pt-10">
    <a href="https://haryx.wecoap.com" class="text-ink text-[17px] no-underline" aria-label="HaryX home page"><Wordmark /></a>
    <a href="https://haryx.wecoap.com" class="text-ink-2 hover:text-ink text-sm">What is HaryX?</a>
  </header>

  <main class="relative z-10 flex flex-1 flex-col justify-center px-4 py-16 sm:px-8 lg:w-[58%] lg:px-14">
    <h1 class="figure text-[clamp(3rem,7vw,6rem)] leading-[0.96]">Your accounts, in&nbsp;any browser.</h1>
    <div class="relative my-4 h-12" aria-hidden="true">
      <div class="absolute inset-y-0 -left-4 w-screen sm:-left-8 lg:-left-14"><TideLine class="h-12" /></div>
    </div>
    <p class="text-ink-2 max-w-[34rem] text-lg">
      See every Claude account's tide, your usage, profiles and machines from HaryX on your Mac or PC. Synced data is end-to-end encrypted and opens only in your tab, with your sync passphrase.
    </p>

    {#if failed}
      <div class="mt-10 max-w-[34rem]" role="alert">
        <p class="font-semibold">Could not reach WeCoAp sign-in.</p>
        <p class="text-ink-2 mt-1">Check your connection, then try again.</p>
        <div class="mt-5"><Button size="lg" onclick={onRetry}>Try again</Button></div>
      </div>
    {:else}
      <div class="mt-10 flex flex-wrap items-center gap-x-6 gap-y-4">
        <Button size="lg" onclick={onSignIn}>Sign in with WeCoAp</Button>
        <a href="https://haryx.wecoap.com/#install-top" class="text-ink-2 hover:text-ink text-[15px]">Get the app first</a>
      </div>
      <p class="text-ink-3 mt-5 max-w-[34rem] text-sm">WeCoAp makes HaryX. Use the same WeCoAp ID you signed in with in the HaryX app.</p>
    {/if}
  </main>

  <Mark class="pointer-events-none absolute top-[14%] right-[6%] -z-10 hidden h-[64vh] max-h-[680px] w-auto lg:block" />
  <Mark class="pointer-events-none absolute -right-8 bottom-6 -z-10 h-40 w-auto opacity-90 lg:hidden" />
</div>
