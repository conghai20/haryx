<script lang="ts">
  // Every synced account as one hairline row: name, newest 5-hour reading, the week's tide, the
  // machines that use it. A row opens the account's sheet.
  import { Input } from "$lib/components/ui/input/index.js";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import Mascot from "./Mascot.svelte";
  import TideStaff from "./TideStaff.svelte";
  import WeekTide from "./WeekTide.svelte";
  import { clock } from "../lib/clock.svelte";
  import { ago, fillOf, type Overview } from "../lib/overview";
  import { machineName, vault } from "../lib/vault.svelte";

  let { view, onOpen }: { view: Overview; onOpen: (id: string) => void } = $props();

  /** A filter appears once the list no longer fits a glance. */
  const FILTER_FROM = 7;
  let query = $state("");
  const shown = $derived(
    query.trim()
      ? view.accounts.filter((a) => `${a.title} ${a.account.email} ${a.account.organizationName}`.toLowerCase().includes(query.trim().toLowerCase()))
      : view.accounts,
  );
  const pct = (v: number | null) => (v === null ? "—" : `${Math.round(v)}%`);
  const tone = (v: number | null) => (fillOf(v) === "red" ? "text-red-text" : fillOf(v) === "amber" ? "text-amber-text" : "");
  const machinesText = (ids: string[]) =>
    ids.length === 0 ? "No synced usage" : ids.length <= 2 ? ids.map((id) => machineName(id, vault.machines)).join(", ") : `${ids.length} machines`;
</script>

{#if view.accounts.length >= FILTER_FROM}
  <div class="mb-4 max-w-sm sm:mb-6">
    <label for="account-filter" class="sr-only">Filter accounts</label>
    <Input id="account-filter" type="search" placeholder="Filter by name, email or organization" bind:value={query} />
  </div>
{/if}

{#if shown.length === 0}
  <p class="text-ink-2 border-rule border-t py-10">No account matches “{query}”.</p>
{:else}
  <div class="text-ink-3 hidden grid-cols-[minmax(0,1.4fr)_minmax(9rem,1fr)_minmax(8rem,1fr)_minmax(0,0.9fr)_1rem] gap-6 pb-3 text-xs font-medium xl:grid" aria-hidden="true">
    <span>Account</span><span>5-hour</span><span>Last 7 days</span><span>Machines</span><span></span>
  </div>
  <ul class="border-rule border-b">
    {#each shown as a (a.id)}
      {@const r = a.reading}
      {@const isLast = view.lastUsed?.id === a.id}
      <li class="border-rule border-t">
        <button
          class="group hover:bg-muted grid w-full grid-cols-[minmax(0,1fr)_auto_1rem] items-center gap-x-4 gap-y-1.5 py-4 text-left transition-colors sm:py-6 xl:grid-cols-[minmax(0,1.4fr)_minmax(9rem,1fr)_minmax(8rem,1fr)_minmax(0,0.9fr)_1rem] xl:gap-6"
          onclick={() => onOpen(a.id)}
          aria-label="{a.title}: details"
        >
          <span class="flex min-w-0 items-start gap-3">
            <Mascot style={a.mascot} class="mt-0.5 size-9" />
            <span class="min-w-0">
            <span class="flex items-center gap-2 text-[17px] font-semibold">
              {#if isLast}<span class="bg-live size-2 shrink-0 rounded-full" aria-hidden="true"></span>{/if}
              <span class="truncate">{a.title}</span>
            </span>
            <span class="text-ink-3 mt-0.5 block truncate text-sm">{a.account.alias ? a.account.email : a.account.organizationName}</span>
            {#if a.account.disabled}<span class="text-ink-2 mt-1 block text-sm">Held out of rotation</span>{/if}
            </span>
          </span>

          <span class="col-start-2 row-start-1 grid min-w-[6.5rem] gap-2 xl:col-start-auto xl:row-start-auto">
            <span class="flex items-baseline justify-end gap-2 xl:justify-start">
              <span class="figure text-[28px] {tone(r?.fiveHourPct ?? null)}">{pct(r?.fiveHourPct ?? null)}</span>
              <span class="text-ink-3 text-xs xl:hidden">5h</span>
            </span>
            <TideStaff pct={r?.fiveHourPct ?? null} live={isLast} label="5-hour window, {pct(r?.fiveHourPct ?? null)}" />
          </span>

          <span class="col-span-2 grid gap-1.5 xl:col-span-1">
            <span class="flex items-baseline justify-between gap-2 text-sm">
              <span class="text-ink-3 hidden sm:inline xl:hidden">Last 7 days</span>
              <span class="font-medium {tone(r?.sevenDayPct ?? null)}">{r?.sevenDayPct == null ? "No week reading" : `Week ${pct(r.sevenDayPct)}`}</span>
            </span>
            <span class="hidden sm:block"><WeekTide points={a.week} now={clock.now} label="Week readings over the last 7 days, now {pct(r?.sevenDayPct ?? null)}" /></span>
          </span>

          <span class="text-ink-2 col-span-2 hidden text-sm sm:block xl:col-span-1">
            {machinesText(a.machineIds)}
            {#if r}<span class="text-ink-3 block text-xs">read {ago(r.at, clock.now)}</span>{/if}
          </span>

          <ChevronRight class="text-ink-3 group-hover:text-ink col-start-3 row-start-1 size-4 transition-transform group-hover:translate-x-0.5 xl:col-start-auto xl:row-start-auto" aria-hidden="true" />
        </button>
      </li>
    {/each}
  </ul>
{/if}
