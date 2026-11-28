<script lang="ts">
  // The split: the hero on the left (the account read last, or the passphrase form), the sections on
  // the right. On phones the hero sits above the sections.
  import { onMount, tick } from "svelte";
  import SearchIcon from "@lucide/svelte/icons/search";
  import { auth } from "./lib/auth.svelte";
  import { clock } from "./lib/clock.svelte";
  import { overview } from "./lib/overview";
  import { SECTIONS, sectionFromHash, type SectionId } from "./lib/sections";
  import { vault } from "./lib/vault.svelte";
  import Hero from "./components/Hero.svelte";
  import TideLine from "./components/brand/TideLine.svelte";
  import AccountSheet from "./components/AccountSheet.svelte";
  import CommandMenu from "./components/CommandMenu.svelte";
  import Accounts from "./views/Accounts.svelte";
  import Usage from "./views/Usage.svelte";
  import Profiles from "./views/Profiles.svelte";
  import Machines from "./views/Machines.svelte";
  import Privacy from "./views/Privacy.svelte";
  import SignedOut from "./views/SignedOut.svelte";
  import Starting from "./views/Starting.svelte";

  let current = $state<SectionId>(sectionFromHash(location.hash));
  let openId = $state<string | null>(null);
  let searching = $state(false);
  let heading = $state<HTMLHeadingElement>();

  const view = $derived(vault.synced ? overview(vault.synced, clock.now) : null);
  const opened = $derived(view?.accounts.find((a) => a.id === openId) ?? null);
  const active = $derived(SECTIONS.find((s) => s.id === current)!);
  const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

  $effect(() => {
    document.title = `${active.label} · HaryX`;
  });

  onMount(() => {
    const stop = vault.track();
    const onHash = async () => {
      current = sectionFromHash(location.hash);
      await tick();
      heading?.focus({ preventScroll: true });
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "k" && (e.metaKey || e.ctrlKey) && auth.status === "signedIn") {
        e.preventDefault();
        searching = !searching;
      }
    };
    addEventListener("hashchange", onHash);
    addEventListener("keydown", onKey);
    void auth.init();
    return () => {
      stop();
      removeEventListener("hashchange", onHash);
      removeEventListener("keydown", onKey);
    };
  });

  $effect(() => {
    if (auth.status !== "signedIn") return;
    void vault.loadConsents().then(() => vault.loadMachines());
  });
</script>

{#if auth.status === "starting"}
  <Starting />
{:else if auth.status === "signedOut" || auth.status === "failed"}
  <SignedOut failed={auth.status === "failed"} onSignIn={auth.signIn} onRetry={() => location.reload() /* keycloak-js initialises once per page */} />
{:else}
  <a href="#content" class="bg-ink text-background sr-only z-50 px-4 py-2 focus:not-sr-only focus:fixed focus:top-3 focus:left-3">Skip to content</a>
  <div class="relative min-h-dvh [--bar:5rem] lg:grid lg:grid-cols-[minmax(26rem,40%)_1fr]">
    <div class="border-rule lg:border-r"><Hero {view} onOpenAccount={(id) => (openId = id)} /></div>
    <div class="pointer-events-none -my-3 h-10 lg:hidden" aria-hidden="true"><TideLine class="h-10" /></div>

    <div class="pointer-events-none fixed inset-x-0 top-[calc(var(--bar)+1rem)] z-30 hidden h-6 lg:block" aria-hidden="true"><TideLine class="h-6" /></div>

    <div class="relative z-10 min-w-0">
      <div class="bg-background border-rule sticky top-0 z-20 border-b px-4 sm:px-8 lg:border-b-0 lg:px-14 lg:pb-10">
        <div class="flex items-end justify-between gap-6 lg:h-(--bar)">
          <nav aria-label="Sections" class="-mb-px flex min-w-0 gap-6 overflow-x-auto pt-4 sm:gap-8 lg:mb-0 lg:pt-0">
            {#each SECTIONS as s (s.id)}
              <a
                href="#{s.id}"
                aria-current={s.id === current ? "page" : undefined}
                class="border-b-2 pb-4 text-[15px] font-medium whitespace-nowrap no-underline transition-colors {s.id === current ? 'border-ink text-ink' : 'text-ink-3 hover:text-ink border-transparent'}"
              >{s.label}</a>
            {/each}
          </nav>
          <button
            class="border-rule text-ink-3 hover:text-ink hover:border-rule-strong mb-3 hidden h-9 shrink-0 items-center gap-3 rounded-[2px] border px-3 text-sm transition-colors md:flex"
            onclick={() => (searching = true)}
          >
            <SearchIcon class="size-4" aria-hidden="true" />
            Jump to
            <kbd class="border-rule rounded-[2px] border px-1.5 font-mono text-[11px]">{isMac ? "⌘" : "Ctrl"} K</kbd>
          </button>
        </div>
      </div>

      <main id="content" class="px-4 pt-6 pb-24 sm:px-8 lg:px-14 lg:pt-8">
        <h2 bind:this={heading} tabindex="-1" class="figure mb-4 text-[clamp(1.75rem,3vw,2.75rem)] outline-none sm:mb-8">
          {active.label}{#if current === "accounts" && view}<span class="text-ink-3 ml-3 align-baseline">{view.accounts.length}</span>{/if}
        </h2>
        {#if current === "accounts"}
          <Accounts {view} onOpen={(id) => (openId = id)} />
        {:else if current === "usage"}
          <Usage />
        {:else if current === "profiles"}
          <Profiles />
        {:else if current === "machines"}
          <Machines />
        {:else}
          <Privacy />
        {/if}
      </main>
    </div>
  </div>

  <AccountSheet account={opened} onClose={() => (openId = null)} />
  <CommandMenu bind:open={searching} sections={SECTIONS} {view} onOpenAccount={(id) => { location.hash = "#accounts"; openId = id; }} />
{/if}
