<script lang="ts">
  // Usage over unlocked data: one card per account for the chosen month, a row per day with every
  // machine's tokens added up, the models inside, and the day's highest 5h/7d readings.
  import State from "./State.svelte";
  import { shortTokens } from "@win/lib/usage";
  import { accountTitle, aggregate, monthsOf, pct } from "../lib/usagestats";
  import type { Synced } from "../lib/synced";

  let { synced }: { synced: Synced } = $props();

  const months = $derived(monthsOf(synced.usage));
  let chosen = $state<string | null>(null);
  const month = $derived(chosen && months.includes(chosen) ? chosen : (months[0] ?? ""));
  const report = $derived(aggregate(synced.usage, month));

  const dayLabel = (day: string) =>
    new Date(`${day}T00:00:00`).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
  const monthLabel = (m: string) => new Date(`${m}-01T00:00:00`).toLocaleDateString(undefined, { month: "long", year: "numeric" });
  const exact = (n: number) => n.toLocaleString();
  const readingTime = (at: number) => new Date(at * 1000).toLocaleString();
</script>

{#if months.length === 0}
  <State kind="empty" title="No synced usage yet" detail="Usage appears here after a machine with Sync turned on runs a sync. It holds tokens per account, day and model, never project names or conversations." />
{:else}
  <div class="controls">
    <label for="usage-month">Month</label>
    <select id="usage-month" value={month} onchange={(e) => (chosen = e.currentTarget.value)}>
      {#each months as m (m)}<option value={m}>{monthLabel(m)}</option>{/each}
    </select>
    <span class="hint">All machines combined · tokens only, no prices</span>
  </div>
  {#if report.length === 0}
    <State kind="empty" title="Nothing in this month" />
  {/if}
  {#each report as acc (acc.account ?? "")}
    <section class="card">
      <header>
        <div class="who">
          <h2>{accountTitle(synced.accounts, acc.account)}</h2>
          {#if acc.account && accountTitle(synced.accounts, acc.account) !== acc.account}<span class="sub">{acc.account}</span>{/if}
        </div>
        <dl class="totals">
          <div><dt>Tokens</dt><dd title={exact(acc.total)}>{shortTokens(acc.total)}</dd></div>
          {#if acc.latest}
            <div title={`Latest reading, ${readingTime(acc.latest.at)}`}><dt>5h</dt><dd>{pct(acc.latest.fiveHourPct)}</dd></div>
            <div title={`Latest reading, ${readingTime(acc.latest.at)}`}><dt>7d</dt><dd>{pct(acc.latest.sevenDayPct)}</dd></div>
          {/if}
          <div><dt>Machines</dt><dd>{acc.machineIds.length}</dd></div>
        </dl>
      </header>
      <ol class="days">
        {#each acc.days as d, i (d.day)}
          <li>
            <details open={i === 0}>
              <summary>
                <span class="day"><time datetime={d.day}>{dayLabel(d.day)}</time></span>
                <span class="num" title={exact(d.total)}>{shortTokens(d.total)}</span>
                <span class="small">{d.replies} replies</span>
                <span class="small q" title="Highest 5-hour and 7-day readings that day">5h {pct(d.fiveHourPct)} · 7d {pct(d.sevenDayPct)}</span>
              </summary>
              {#if d.models.length}
                <table>
                  <thead><tr><th scope="col">Model</th><th scope="col">Total</th><th scope="col">In</th><th scope="col">Out</th><th scope="col">Cache read</th><th scope="col">Cache write</th></tr></thead>
                  <tbody>
                    {#each d.models as m (m.model)}
                      <tr>
                        <th scope="row">{m.model}</th>
                        <td data-label="Total" title={exact(m.total)}>{shortTokens(m.total)}</td>
                        <td data-label="In">{shortTokens(m.input)}</td>
                        <td data-label="Out">{shortTokens(m.output)}</td>
                        <td data-label="Cache read">{shortTokens(m.cacheRead)}</td>
                        <td data-label="Cache write">{shortTokens(m.cacheWrite)}</td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              {:else}
                <p class="small pad">Readings only; no tokens recorded this day.</p>
              {/if}
            </details>
          </li>
        {/each}
      </ol>
    </section>
  {/each}
{/if}

<style>
  .controls { display: flex; flex-wrap: wrap; gap: .5rem .75rem; align-items: center; margin-bottom: 1rem; max-width: 52rem; }
  label { font-weight: 600; font-size: .9rem; }
  select { font: inherit; color: inherit; background: var(--surface); border: 1px solid var(--separator); border-radius: var(--radius-control); padding: .4rem .6rem; min-height: 40px; max-width: 100%; }
  .hint, .small, .sub { color: var(--label-2); font-size: .85rem; }
  .card { background: var(--surface); border: 1px solid var(--separator); border-radius: var(--radius-card); padding: 1rem 1.25rem; max-width: 52rem; margin-bottom: .75rem; min-width: 0; }
  header { display: flex; flex-wrap: wrap; gap: .5rem 1.5rem; justify-content: space-between; align-items: flex-start; margin-bottom: .5rem; }
  .who { display: grid; min-width: 0; flex: 1 1 14rem; }
  h2, .sub { overflow-wrap: anywhere; margin: 0; }
  .totals { display: flex; flex-wrap: wrap; gap: 1.25rem; margin: 0; }
  .totals div { display: grid; }
  dt { font-size: .75rem; color: var(--label-2); }
  dd { margin: 0; font: 600 1.15rem/1.2 var(--font-display); font-variant-numeric: tabular-nums; }
  .days { list-style: none; margin: 0; padding: 0; }
  .days li { border-top: 1px solid var(--separator); }
  summary { display: flex; flex-wrap: wrap; gap: .25rem .9rem; align-items: baseline; padding: .55rem 0; cursor: pointer; list-style-position: outside; }
  summary::marker { color: var(--label-2); }
  .day { min-width: 7.5rem; font-weight: 600; }
  .num { font-variant-numeric: tabular-nums; min-width: 4rem; }
  .q { margin-left: auto; }
  table { width: 100%; border-collapse: collapse; font-size: .85rem; margin: 0 0 .6rem; font-variant-numeric: tabular-nums; }
  th, td { text-align: right; padding: .25rem .4rem; }
  th[scope="row"], thead th:first-child { text-align: left; overflow-wrap: anywhere; font-weight: 500; }
  thead th { color: var(--label-2); font-weight: 500; font-size: .75rem; }
  tbody tr:nth-child(odd) { background: var(--fill); }
  .pad { padding: 0 0 .6rem; margin: 0; }
  /* Narrow: each model becomes a small block, labels beside values. */
  @media (max-width: 640px) {
    .card { padding: .9rem; }
    .q { margin-left: 0; }
    thead { display: none; }
    table, tbody, tr, th, td { display: block; }
    tr { padding: .4rem .5rem; border-radius: var(--radius-control); }
    tbody tr { display: grid; grid-template-columns: 1fr 1fr; gap: .1rem .75rem; }
    th[scope="row"] { grid-column: 1 / -1; padding: 0 0 .1rem; font-weight: 600; }
    td { padding: 0; display: flex; justify-content: space-between; gap: .5rem; }
    td::before { content: attr(data-label); color: var(--label-2); }
  }
</style>
