<script lang="ts">
  // The left half of the split: the homepage hero turned console. Unlocked, it sets the account
  // read most recently at display size; locked, it is the passphrase form. Every figure says when
  // and on which machine it was read: synced readings are history, never live.
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Skeleton } from "$lib/components/ui/skeleton/index.js";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import Mark from "./brand/Mark.svelte";
  import Wordmark from "./brand/Wordmark.svelte";
  import TideStaff from "./TideStaff.svelte";
  import Mascot from "./Mascot.svelte";
  import UnlockForm from "./UnlockForm.svelte";
  import { auth } from "../lib/auth.svelte";
  import { accountConsoleUrl } from "../lib/config";
  import { clock } from "../lib/clock.svelte";
  import { ago, breakable, fillOf, splitEmail, type Overview } from "../lib/overview";
  import { syncLock } from "../lib/synclock";
  import { machineName, vault } from "../lib/vault.svelte";

  let { view, onOpenAccount }: { view: Overview | null; onOpenAccount: (id: string) => void } = $props();

  const last = $derived(view?.lastUsed ?? null);
  const pct = (v: number | null) => (v === null ? "No reading" : `${Math.round(v)}%`);
  const initial = $derived((auth.email ?? "?").slice(0, 1).toUpperCase());
  const machineCount = $derived(new Set(view?.accounts.flatMap((a) => a.machineIds) ?? []).size);
</script>

<header class="relative isolate flex flex-col overflow-hidden px-4 pt-5 pb-6 sm:px-8 lg:sticky lg:top-0 lg:h-dvh lg:px-14 lg:pt-10 lg:pb-12">
  <div class="flex items-center justify-between gap-4">
    <a href="https://haryx.wecoap.com" class="text-ink text-[17px] no-underline" aria-label="HaryX home page"><Wordmark /></a>
    <DropdownMenu.Root>
      <DropdownMenu.Trigger>
        {#snippet child({ props })}
          <button {...props} class="text-ink-2 hover:text-ink flex h-10 max-w-[min(60vw,15rem)] min-w-0 items-center gap-2 rounded-[2px] text-sm" aria-label="Account menu for {auth.email ?? 'you'}">
            <span class="bg-ink text-background grid size-7 shrink-0 place-items-center rounded-[2px] text-xs font-semibold" aria-hidden="true">{initial}</span>
            <span class="hidden truncate sm:inline">{auth.email ?? "Signed in"}</span>
            <ChevronDown class="size-4 shrink-0" aria-hidden="true" />
          </button>
        {/snippet}
      </DropdownMenu.Trigger>
      <DropdownMenu.Content align="end" class="w-64">
        <DropdownMenu.Label class="text-ink-3 truncate font-normal">{auth.email ?? "Signed in with WeCoAp"}</DropdownMenu.Label>
        <DropdownMenu.Separator />
        {#if syncLock.status === "unlocked"}
          <DropdownMenu.Item onSelect={() => syncLock.lock()}>Lock synced data</DropdownMenu.Item>
        {/if}
        <DropdownMenu.Item onSelect={() => window.open(accountConsoleUrl(), "_blank", "noopener")}>WeCoAp account</DropdownMenu.Item>
        <DropdownMenu.Item onSelect={() => (location.hash = "#privacy")}>Privacy center</DropdownMenu.Item>
        <DropdownMenu.Separator />
        <DropdownMenu.Item onSelect={() => auth.signOut()}>Sign out of HaryX web</DropdownMenu.Item>
      </DropdownMenu.Content>
    </DropdownMenu.Root>
  </div>

  <div class="relative z-10 mt-8 flex flex-1 flex-col lg:mt-[12vh]">
    {#if vault.state === "loading" || vault.state === "decoding"}
      <div class="grid max-w-md gap-4" role="status" aria-label="Loading">
        <Skeleton class="h-16 w-3/4" />
        <Skeleton class="h-4 w-1/2" />
        <Skeleton class="mt-6 h-12 w-1/3" />
        <Skeleton class="h-1.5 w-full" />
      </div>
    {:else if vault.state === "error"}
      <h1 class="figure text-[clamp(2.25rem,4.2vw,3.5rem)]">Could not reach HaryX.</h1>
      <p class="text-ink-2 mt-4 max-w-md">Your consents did not load, so nothing can be shown yet. Your data is safe; try again.</p>
      <div class="mt-6"><Button onclick={() => vault.loadConsents()}>Try again</Button></div>
    {:else if vault.state === "syncOff"}
      <h1 class="figure text-[clamp(2.25rem,4.2vw,3.5rem)]">Sync is off.</h1>
      <p class="text-ink-2 mt-4 max-w-md">Accounts, usage and profiles come from Sync, end-to-end encrypted. Allow Sync in the Privacy center, then turn it on in the HaryX app on your Mac or PC. Machines and privacy work without it.</p>
      <div class="mt-6"><Button href="#privacy">Open Privacy center</Button></div>
    {:else if vault.state === "locked" || vault.state === "unlocking"}
      <h1 class="figure text-[clamp(2.5rem,5vw,4.5rem)]">Unlock your accounts.</h1>
      <p class="text-ink-2 mt-4 mb-8 max-w-md">Your accounts, usage and profiles are end-to-end encrypted. Enter the sync passphrase you set in the HaryX app to read them here.</p>
      <div class="max-w-md"><UnlockForm /></div>
    {:else if vault.state === "decodeFailed"}
      <h1 class="figure text-[clamp(2.25rem,4.2vw,3.5rem)]">Could not open your data.</h1>
      <p class="text-ink-2 mt-4 max-w-md">The passphrase was right but the synced data did not decrypt. Lock and unlock again to retry.</p>
      <div class="mt-6"><Button onclick={() => syncLock.lock()}>Lock</Button></div>
    {:else if last && last.reading}
      {@const r = last.reading}
      {@const email = splitEmail(last.title)}
      <Mascot style={last.mascot} class="mb-4 hidden size-16 lg:block" />
      <h1 class="figure relative">
        <span class="bg-live absolute top-[0.42em] -left-3 size-2 rounded-full lg:hidden" aria-hidden="true"></span>
        <button class="hover:text-ink-2 line-clamp-1 w-full text-left text-[clamp(2rem,9vw,2.5rem)] leading-[1.02] transition-colors lg:line-clamp-2 lg:text-[clamp(3rem,6vw,5.5rem)]" onclick={() => onOpenAccount(last.id)} aria-label="{last.title}: details">{#each breakable(email ? email.local : last.title) as part, k (k)}<wbr />{part}{/each}</button>
      </h1>
      {#if email || last.account.alias}
        <p class="text-ink-2 mt-2 truncate text-base lg:mt-3 lg:text-lg">{email ? email.domain : last.account.email}</p>
      {/if}

      <dl class="mt-6 grid max-w-md grid-cols-2 gap-x-8 gap-y-6 lg:mt-[clamp(1.75rem,5vh,3.5rem)] lg:grid-cols-1 lg:gap-7">
        <div class="grid gap-2 lg:gap-3">
          <dt class="text-ink-2 text-sm">5-hour window</dt>
          <dd class="figure {r.fiveHourPct === null ? 'text-ink-3 text-lg tracking-normal' : 'text-[32px] lg:text-[clamp(2.75rem,4.6vw,4rem)]'} {fillOf(r.fiveHourPct) === 'red' ? 'text-red-text' : fillOf(r.fiveHourPct) === 'amber' ? 'text-amber-text' : ''}">{pct(r.fiveHourPct)}</dd>
          <dd><TideStaff pct={r.fiveHourPct} size="lg" live label="5-hour window, {pct(r.fiveHourPct)}" /></dd>
        </div>
        <div class="grid gap-2">
          <dt class="text-ink-2 text-sm">Week</dt>
          <dd class="figure {r.sevenDayPct === null ? 'text-ink-3 text-lg tracking-normal' : 'text-[32px] lg:text-[28px]'} {fillOf(r.sevenDayPct) === 'red' ? 'text-red-text' : fillOf(r.sevenDayPct) === 'amber' ? 'text-amber-text' : ''}">{pct(r.sevenDayPct)}</dd>
          <dd><TideStaff pct={r.sevenDayPct} size="sm" label="Week, {pct(r.sevenDayPct)}" /></dd>
        </div>
      </dl>
      <p class="text-ink-3 mt-5 max-w-md text-sm lg:mt-8">
        Read <time datetime={new Date(r.at * 1000).toISOString()}>{ago(r.at, clock.now)}</time> on {machineName(r.machineId, vault.machines)}, the newest reading of any account.
        <span class="hidden lg:inline">{view!.accounts.length} {view!.accounts.length === 1 ? "account" : "accounts"} · {machineCount} {machineCount === 1 ? "machine" : "machines"}.</span>
      </p>
      <p class="text-ink-3 mt-3 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm" role="status">
        Unlocked in this tab. Locks after 15 minutes idle.
        <button class="text-ink hover:text-live-text font-medium underline underline-offset-3" onclick={() => syncLock.lock()}>Lock now</button>
      </p>
    {:else}
      <h1 class="figure text-[clamp(2.5rem,5vw,4.5rem)]">No readings yet.</h1>
      <p class="text-ink-2 mt-4 max-w-md">
        {view?.accounts.length ? `${view.accounts.length} synced ${view.accounts.length === 1 ? "account" : "accounts"}, but no machine has synced a quota reading yet.` : "Nothing has synced yet."}
        Readings arrive when a Mac or PC with Sync on runs a sync.
      </p>
    {/if}
  </div>

  <Mark class="pointer-events-none absolute -bottom-20 -left-16 -z-10 hidden h-[min(46vh,420px)] w-auto opacity-[0.14] lg:block" />
</header>
