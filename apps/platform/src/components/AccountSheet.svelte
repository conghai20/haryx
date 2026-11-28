<script lang="ts">
  // One account in full, in a side sheet: identity and settings, the newest readings with where and
  // when they were taken, the week's tide, the machines that use it and this month's tokens.
  import * as Sheet from "$lib/components/ui/sheet/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { shortTokens } from "@win/lib/usage";
  import TideStaff from "./TideStaff.svelte";
  import Mascot from "./Mascot.svelte";
  import WeekTide from "./WeekTide.svelte";
  import { clock } from "../lib/clock.svelte";
  import { ago, fillOf, type AccountOverview } from "../lib/overview";
  import { aggregate, monthsOf, normEmail } from "../lib/usagestats";
  import { machineName, vault } from "../lib/vault.svelte";

  let { account, onClose }: { account: AccountOverview | null; onClose: () => void } = $props();

  const pct = (v: number | null) => (v === null ? "No reading" : `${Math.round(v)}%`);
  const tone = (v: number | null) => (fillOf(v) === "red" ? "text-red-text" : fillOf(v) === "amber" ? "text-amber-text" : "");
  const month = $derived(vault.synced ? (monthsOf(vault.synced.usage)[0] ?? null) : null);
  const tokens = $derived.by(() => {
    if (!account || !vault.synced || !month) return null;
    const email = normEmail(account.account.email);
    return aggregate(vault.synced.usage, month).find((u) => u.account === email)?.total ?? 0;
  });
  const monthLabel = (m: string) => new Date(`${m}-01T00:00:00`).toLocaleDateString(undefined, { month: "long" });
</script>

<Sheet.Root open={account !== null} onOpenChange={(open) => { if (!open) onClose(); }}>
  <Sheet.Content side="right" class="bg-background w-full gap-0 overflow-y-auto p-0 sm:max-w-md">
    {#if account}
      {@const a = account}
      {@const r = a.reading}
      <Sheet.Header class="border-rule border-b px-6 pt-8 pr-14 pb-6 sm:px-8 sm:pr-14">
        <Mascot style={a.mascot} class="mb-3 size-14" />
        <Sheet.Title class="figure text-[40px] break-words">{a.title}</Sheet.Title>
        <Sheet.Description class="text-ink-2 break-all">{a.account.email}</Sheet.Description>
        <p class="text-ink-3 mt-2 text-sm">
          {[a.account.organizationName, a.account.priority !== null ? `Switch order ${a.account.priority}` : "No switch order set", a.account.disabled ? "Held out of rotation" : ""].filter(Boolean).join(" · ")}
        </p>
      </Sheet.Header>

      <div class="grid gap-8 px-6 py-8 sm:px-8">
        {#if r}
          <section class="grid gap-6" aria-labelledby="sheet-readings">
            <h3 id="sheet-readings" class="sr-only">Newest readings</h3>
            <div class="grid gap-2">
              <div class="flex items-baseline justify-between"><span class="text-ink-2 text-sm">5-hour window</span><span class="figure text-[32px] {tone(r.fiveHourPct)}">{pct(r.fiveHourPct)}</span></div>
              <TideStaff pct={r.fiveHourPct} size="lg" label="5-hour window, {pct(r.fiveHourPct)}" />
            </div>
            <div class="grid gap-2">
              <div class="flex items-baseline justify-between"><span class="text-ink-2 text-sm">Week</span><span class="figure text-[32px] {tone(r.sevenDayPct)}">{pct(r.sevenDayPct)}</span></div>
              <TideStaff pct={r.sevenDayPct} size="md" label="Week, {pct(r.sevenDayPct)}" />
            </div>
            <p class="text-ink-3 text-sm">Read <time datetime={new Date(r.at * 1000).toISOString()}>{ago(r.at, clock.now)}</time> on {machineName(r.machineId, vault.machines)}.</p>
          </section>
        {:else}
          <p class="text-ink-2">No machine has synced a reading for this account yet.</p>
        {/if}

        <section class="grid gap-3" aria-labelledby="sheet-week">
          <h3 id="sheet-week" class="text-sm font-semibold">Last 7 days</h3>
          {#if a.week.length >= 2}
            <WeekTide points={a.week} now={clock.now} height={64} label="Week readings over the last 7 days" />
            <div class="text-ink-3 flex justify-between text-xs"><span>7 days ago</span><span>Now</span></div>
          {:else}
            <p class="text-ink-3 text-sm">Not enough weekly readings in the last 7 days to draw the week.</p>
          {/if}
        </section>

        <section class="grid gap-3" aria-labelledby="sheet-machines">
          <h3 id="sheet-machines" class="text-sm font-semibold">Machines</h3>
          {#if a.machineIds.length}
            <ul class="border-rule border-b">
              {#each a.machineIds as id (id)}
                <li class="border-rule flex justify-between gap-4 border-t py-3 text-sm">
                  <span>{machineName(id, vault.machines)}</span>
                  {#if r?.machineId === id}<span class="text-ink-3">newest reading</span>{/if}
                </li>
              {/each}
            </ul>
          {:else}
            <p class="text-ink-3 text-sm">No machine has synced usage for this account.</p>
          {/if}
        </section>

        {#if tokens !== null && month}
          <section class="grid gap-3" aria-labelledby="sheet-tokens">
            <h3 id="sheet-tokens" class="text-sm font-semibold">Tokens in {monthLabel(month)}</h3>
            <p class="figure text-[32px]" title={tokens.toLocaleString()}>{shortTokens(tokens)}</p>
            <div><Button variant="outline" href="#usage" onclick={onClose}>Open usage</Button></div>
          </section>
        {/if}
      </div>
    {/if}
  </Sheet.Content>
</Sheet.Root>
