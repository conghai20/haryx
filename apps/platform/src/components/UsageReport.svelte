<script lang="ts">
  // Usage over unlocked data for one month: an account per hairline row, its tokens and the day's
  // highest readings, opening into a row per day and the models inside. All machines are added up.
  import * as Select from "$lib/components/ui/select/index.js";
  import * as Table from "$lib/components/ui/table/index.js";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import State from "./State.svelte";
  import { shortTokens } from "@win/lib/usage";
  import { accountTitle, aggregate, monthsOf, type AccountUsage } from "../lib/usagestats";
  import { fillOf } from "../lib/overview";
  import type { Synced } from "../lib/synced";

  let { synced }: { synced: Synced } = $props();

  const months = $derived(monthsOf(synced.usage));
  let chosen = $state<string | null>(null);
  const month = $derived(chosen && months.includes(chosen) ? chosen : (months[0] ?? ""));
  const report = $derived(aggregate(synced.usage, month));
  const monthTotal = $derived(report.reduce((n, a) => n + a.total, 0));
  let open = $state<Record<string, boolean>>({});
  let openDay = $state<string | null>(null);
  const keyOf = (a: AccountUsage) => a.account ?? "";
  const isOpen = (a: AccountUsage, i: number) => open[keyOf(a)] ?? (i === 0 && report.length <= 3);

  const dayLabel = (day: string) => new Date(`${day}T00:00:00`).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
  const monthLabel = (m: string) => new Date(`${m}-01T00:00:00`).toLocaleDateString(undefined, { month: "long", year: "numeric" });
  const exact = (n: number) => n.toLocaleString();
  const pct = (v: number | null) => (v === null ? "No reading" : `${Math.round(v)}%`);
  const tone = (v: number | null) => (fillOf(v) === "red" ? "text-red-text" : fillOf(v) === "amber" ? "text-amber-text" : "");
</script>

{#if months.length === 0}
  <State kind="empty" title="No synced usage yet" detail="Usage appears here after a machine with Sync on runs a sync. It holds tokens per account, day and model, never project names or conversations." />
{:else}
  <div class="mb-10 flex flex-wrap items-end gap-x-10 gap-y-6">
    <div class="grid gap-2">
      <span id="usage-month-label" class="text-ink-2 text-sm">Month</span>
      <Select.Root type="single" value={month} onValueChange={(v) => (chosen = v)}>
        <Select.Trigger class="min-w-52" aria-labelledby="usage-month-label">{monthLabel(month)}</Select.Trigger>
        <Select.Content>
          {#each months as m (m)}<Select.Item value={m} label={monthLabel(m)}>{monthLabel(m)}</Select.Item>{/each}
        </Select.Content>
      </Select.Root>
    </div>
    <div class="grid gap-1">
      <span class="text-ink-2 text-sm">Tokens, all accounts</span>
      <span class="figure text-[40px]" title={exact(monthTotal)}>{shortTokens(monthTotal)}</span>
    </div>
    <p class="text-ink-3 pb-1 text-sm">All machines added up · tokens only, no prices</p>
  </div>

  {#if report.length === 0}
    <State kind="empty" title="Nothing in {monthLabel(month)}" />
  {:else}
    <ul class="border-rule max-w-5xl border-b">
      {#each report as acc, i (keyOf(acc))}
        {@const expanded = isOpen(acc, i)}
        {@const title = accountTitle(synced.accounts, acc.account)}
        <li class="border-rule border-t">
          <button
            class="hover:bg-muted grid w-full grid-cols-[minmax(0,1fr)_auto_1rem] items-center gap-x-6 gap-y-2 py-6 text-left transition-colors md:grid-cols-[minmax(0,1.4fr)_8rem_minmax(13rem,auto)_1rem]"
            aria-expanded={expanded}
            aria-controls="usage-{i}"
            onclick={() => (open = { ...open, [keyOf(acc)]: !expanded })}
          >
            <span class="min-w-0">
              <span class="block truncate text-[17px] font-semibold">{title}</span>
              <span class="text-ink-3 block truncate text-sm">{acc.account && title !== acc.account ? acc.account : `${acc.machineIds.length} ${acc.machineIds.length === 1 ? "machine" : "machines"}`}</span>
            </span>
            <span class="figure text-right text-[28px] md:text-left" title={exact(acc.total)}>{shortTokens(acc.total)}</span>
            <span class="text-ink-2 col-span-2 text-sm md:col-span-1">
              {#if acc.latest}<span class="whitespace-nowrap {tone(acc.latest.fiveHourPct)}">5h {acc.latest.fiveHourPct === null ? "no reading" : `${Math.round(acc.latest.fiveHourPct)}%`}</span><span class="text-ink-3 mx-1.5" aria-hidden="true">·</span><span class="whitespace-nowrap {tone(acc.latest.sevenDayPct)}">Week {acc.latest.sevenDayPct === null ? "no reading" : `${Math.round(acc.latest.sevenDayPct)}%`}</span>{:else}No readings{/if}
            </span>
            <ChevronDown class="text-ink-3 col-start-3 row-start-1 size-4 transition-transform md:col-start-auto md:row-start-auto {expanded ? 'rotate-180' : ''}" aria-hidden="true" />
          </button>

          {#if expanded}
            <div id="usage-{i}" class="pb-8">
              <Table.Root>
                <Table.Header>
                  <Table.Row class="border-rule hover:bg-transparent">
                    <Table.Head class="text-ink-3 pl-0 font-medium">Day</Table.Head>
                    <Table.Head class="text-ink-3 text-right font-medium">Tokens</Table.Head>
                    <Table.Head class="text-ink-3 text-right font-medium">Replies</Table.Head>
                    <Table.Head class="text-ink-3 hidden text-right font-medium sm:table-cell">Highest 5h</Table.Head>
                    <Table.Head class="text-ink-3 hidden text-right font-medium sm:table-cell">Highest week</Table.Head>
                    <Table.Head class="text-ink-3 hidden pr-0 font-medium lg:table-cell">Top model</Table.Head>
                  </Table.Row>
                </Table.Header>
                <Table.Body>
                  {#each acc.days as d (d.day)}
                    {@const dayKey = `${keyOf(acc)}|${d.day}`}
                    <Table.Row class="border-rule">
                      <Table.Cell class="pl-0">
                        <button class="hover:text-ink-2 flex items-center gap-2 text-left" aria-expanded={openDay === dayKey} onclick={() => (openDay = openDay === dayKey ? null : dayKey)} disabled={d.models.length === 0}>
                          <ChevronDown class="text-ink-3 size-3.5 transition-transform {openDay === dayKey ? 'rotate-180' : ''} {d.models.length === 0 ? 'invisible' : ''}" aria-hidden="true" />
                          <time datetime={d.day}>{dayLabel(d.day)}</time>
                        </button>
                      </Table.Cell>
                      <Table.Cell class="text-right font-medium" title={exact(d.total)}>{shortTokens(d.total)}</Table.Cell>
                      <Table.Cell class="text-ink-2 text-right">{d.replies}</Table.Cell>
                      <Table.Cell class="hidden text-right sm:table-cell {tone(d.fiveHourPct)}">{d.fiveHourPct === null ? "" : `${Math.round(d.fiveHourPct)}%`}</Table.Cell>
                      <Table.Cell class="hidden text-right sm:table-cell {tone(d.sevenDayPct)}">{d.sevenDayPct === null ? "" : `${Math.round(d.sevenDayPct)}%`}</Table.Cell>
                      <Table.Cell class="text-ink-2 hidden max-w-56 truncate pr-0 lg:table-cell">
                        {d.models[0]?.model ?? "Readings only"}{#if d.models.length > 1}<span class="text-ink-3"> +{d.models.length - 1}</span>{/if}
                      </Table.Cell>
                    </Table.Row>
                    {#if openDay === dayKey}
                      <Table.Row class="border-rule hover:bg-transparent">
                        <Table.Cell colspan={6} class="bg-muted px-4 py-4 whitespace-normal">
                          <dl class="grid gap-4 text-sm">
                            {#each d.models as m (m.model)}
                              <div class="grid gap-x-6 gap-y-1 sm:grid-cols-[minmax(0,1.5fr)_repeat(4,minmax(0,1fr))]">
                                <dt class="truncate font-medium">{m.model}</dt>
                                <dd class="text-ink-2 flex gap-1.5"><span class="text-ink-3">In</span>{shortTokens(m.input)}</dd>
                                <dd class="text-ink-2 flex gap-1.5"><span class="text-ink-3">Out</span>{shortTokens(m.output)}</dd>
                                <dd class="text-ink-2 flex gap-1.5"><span class="text-ink-3">Cache read</span>{shortTokens(m.cacheRead)}</dd>
                                <dd class="text-ink-2 flex gap-1.5"><span class="text-ink-3">Cache write</span>{shortTokens(m.cacheWrite)}</dd>
                              </div>
                            {/each}
                          </dl>
                        </Table.Cell>
                      </Table.Row>
                    {/if}
                  {/each}
                </Table.Body>
              </Table.Root>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
{/if}
