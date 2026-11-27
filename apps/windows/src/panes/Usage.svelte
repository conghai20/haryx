<script lang="ts">
  // Usage. First, will the week last: the CLI's rotation verdict and each account's Week Tide
  // (WeekSection). Then where it went: tokens (or API-price cost) per day by account and the projects
  // and sessions that used the most, from `haryx ai usage --json` once usage history is on. Kept on this PC unless
  // the Sync consent is given and sync is on: then daily totals per account and model sync (`usageKept`).
  import { api } from "../lib/api";
  import { actions } from "../lib/actions";
  import { describeError } from "../lib/errors";
  import Icon from "../components/Icon.svelte";
  import WeekSection from "../components/WeekSection.svelte";
  import { usageKept } from "../lib/consent";
  import { haryxAccount } from "../lib/haryx.svelte";
  import { store } from "../lib/store.svelte";
  import { displayName } from "../lib/tide";
  import {
    accountFill,
    money,
    shortTokens,
    stackedBars,
    type HistoryStatus,
    type UsageReport,
  } from "../lib/usage";

  const ranges = [
    { id: "24h", label: "24 hours" },
    { id: "7d", label: "7 days" },
    { id: "30d", label: "30 days" },
    { id: "all", label: "Everything kept" },
  ];
  const W = 640;
  const H = 160;

  let range = $state("7d");
  let account = $state<number | null>(null);
  let showCost = $state(false);
  let loading = $state(false);
  let error = $state<{ text: string; detail: string } | null>(null);
  let status = $state<HistoryStatus | null>(null);
  let byAccount = $state<UsageReport | null>(null);
  let byDay = $state<Record<number, UsageReport>>({});
  let projects = $state<UsageReport | null>(null);
  let sessions = $state<UsageReport | null>(null);

  function name(n: number): string {
    const a = store.accounts.find((x) => x.number === n);
    return a ? displayName(a) : `Account ${n}`;
  }

  // Bumped on every load; a load that is no longer the latest drops its results.
  let generation = 0;

  async function load() {
    const mine = ++generation;
    const current = () => mine === generation;
    loading = true;
    try {
      const base = ["--since", range, ...(account != null ? ["--account", String(account)] : [])];
      const first = await api.json<UsageReport>(["ai", "usage", "report", "--by", "account", ...base, "--json"]);
      if (!current()) return;
      status = first.history ?? null;
      if (!status?.enabled || status.needsChoice) {
        byAccount = null;
        return;
      }
      const numbers = first.rows.map((r) => Number(r.key)).filter((n) => Number.isFinite(n));
      const [p, s, ...days] = await Promise.all([
        api.json<UsageReport>(["ai", "usage", "report", "--by", "project", ...base, "--json"]),
        api.json<UsageReport>(["ai", "usage", "report", "--by", "session", ...base, "--json"]),
        ...numbers.map((n) =>
          api.json<UsageReport>(["ai", "usage", "report", "--by", "day", "--since", range, "--account", String(n), "--json"]),
        ),
      ]);
      if (!current()) return;
      byAccount = first;
      projects = p;
      sessions = s;
      const perDay: Record<number, UsageReport> = {};
      numbers.forEach((n, i) => {
        const report = days[i];
        if (report) perDay[n] = report;
      });
      byDay = perDay;
      error = null;
    } catch (e) {
      if (current()) error = describeError(e, "Couldn't read the usage history");
    } finally {
      if (current()) loading = false;
    }
  }

  async function choose(all: boolean) {
    if (await store.run("Starting your usage history", ["ai", "usage", "index", "--import", all ? "all" : "none", "--json"]))
      await load();
  }

  async function keepDays(days: string) {
    if (await store.run("Saving", ["config", "set", "history.keepDays", days], { success: "Saved" })) await load();
  }

  async function turnOn() {
    if (await store.run("Turning on usage history", ["config", "set", "history.enabled", "true"], { success: "Usage history is on" }))
      await load();
  }

  /** One fixed colour per account, the same in every chart and table (`ACCOUNT_PALETTE`). */
  function lineColor(n: number): string {
    return accountFill(n);
  }

  /** Five evenly spaced y-axis figures for the per-day chart, top first. */
  function axisTicks(max: number): { y: number; label: string }[] {
    return [1, 0.75, 0.5, 0.25, 0].map((f) => ({
      y: H - f * H,
      label: showCost ? `$${shortTokens(max * f)}` : shortTokens(max * f),
    }));
  }

  $effect(() => {
    void range;
    void account;
    load();
  });

  let bars = $derived(stackedBars(byDay, showCost, W, H));
</script>

<WeekSection />
<hr class="divider" />
<h2 class="section">Where it went</h2>
{#if status?.needsChoice}
  <section class="choice">
    <p class="secondary">
      HaryX can read your past Claude Code transcripts to show how each account, project and session used
      its quota. It reads token counts only, never the conversations. {usageKept(haryxAccount.usageSyncs)}
    </p>
    <div class="row">
      <button class="primary" onclick={() => choose(true)}>Read past transcripts</button>
      <button onclick={() => choose(false)}>Start from now</button>
    </div>
  </section>
{:else if status && !status.enabled}
  <section class="choice">
    <p class="secondary">
      Which accounts, projects and sessions used your quota appears here once usage history is on. HaryX reads
      token counts from Claude Code's transcripts on this PC; no conversation text is stored.
    </p>
    <div class="row"><button class="primary" onclick={turnOn}>Turn on usage history</button></div>
  </section>
{:else}
  <div class="row controls">
    <select aria-label="Range" bind:value={range}>
      {#each ranges as r (r.id)}<option value={r.id}>{r.label}</option>{/each}
    </select>
    <select aria-label="Account" value={account == null ? "" : String(account)} onchange={(e) => (account = e.currentTarget.value ? Number(e.currentTarget.value) : null)}>
      <option value="">All accounts</option>
      {#each store.accounts as a (a.number)}<option value={String(a.number)}>{a.number}. {displayName(a)}</option>{/each}
    </select>
    <label class="check"><input type="checkbox" bind:checked={showCost} /> Show API cost</label>
    <span class="spacer"></span>
    {#if loading}<span class="caption secondary" role="status">Reading…</span>{/if}
    <button class="plain icon-btn" aria-label="Refresh usage" title="Read new transcript lines and refresh" onclick={load}>
      <Icon name="refresh" />
    </button>
  </div>
  {#if error}
    <div class="banner error" role="alert" title={error.detail}>
      <span>{error.text}</span>
      <button class="mini" onclick={load}>Try again</button>
    </div>
  {:else if loading && !byAccount}
    <p class="secondary" role="status">Reading your usage history…</p>
  {/if}

  {#if byAccount}
    <div class="stats">
      <div><strong>{shortTokens(byAccount.total.tokens)}</strong><span class="caption secondary">Tokens</span></div>
      <div><strong>{money(byAccount.total.cost)}</strong><span class="caption secondary">At API prices</span></div>
      <div><strong>{byAccount.total.replies}</strong><span class="caption secondary">Replies</span></div>
      <div><strong>{shortTokens(byAccount.total.output)}</strong><span class="caption secondary">Output tokens</span></div>
    </div>

    <h2>{showCost ? "API cost per day" : "Tokens per day"}</h2>
    {#if bars.bars.length === 0}
      <p class="caption secondary">No usage in this range.</p>
    {:else}
      <svg viewBox="-40 -4 {W + 40} {H + 20}" class="chart" role="img" aria-label="Usage per day by account">
        {#each axisTicks(bars.max) as t (t.y)}
          <line x1="0" x2={W} y1={t.y} y2={t.y} class="grid" />
          <text x="-6" y={t.y + 3} class="axis">{t.label}</text>
        {/each}
        {#each bars.bars as b (b.day)}
          {#each b.segments as s (s.account)}
            <rect x={b.x} y={s.y} width={b.width} height={s.height} fill={lineColor(s.account)}>
              <title>{b.day} · {name(s.account)} · {showCost ? money(s.value) : shortTokens(s.value)}</title>
            </rect>
          {/each}
          {#if bars.bars.length <= 31}<text x={b.x + b.width / 2} y={H + 12} class="tick">{b.day.slice(5)}</text>{/if}
        {/each}
      </svg>
    {/if}

    <h2>By account</h2>
    <table>
      <thead>
        <tr class="caption tertiary">
          <th>Account</th><th class="num">Tokens</th><th class="num">At API prices</th><th class="num">Share</th>
        </tr>
      </thead>
      <tbody>
        {#each byAccount.rows as r (r.key)}
          {@const n = r.key && r.key !== "unknown" ? Number(r.key) : null}
          <tr>
            <td class="truncate">{#if n != null}<span class="swatch" style:background={lineColor(n)}></span>{/if}{n != null ? name(n) : "Unknown account"}{#if r.approximate} <span class="tertiary" title="Account worked out from the switch log, so approximate">~</span>{/if}</td>
            <td class="num">{shortTokens(r.tokens)}</td>
            <td class="num">{money(r.cost)}</td>
            <td class="num secondary">{r.share}%</td>
          </tr>
        {/each}
      </tbody>
    </table>

    {#each [{ title: "Top projects", report: projects, folder: true }, { title: "Top sessions", report: sessions, folder: false }] as t (t.title)}
      {#if t.report}
        <h2>{t.title}</h2>
        <table>
          <thead>
            <tr class="caption tertiary">
              <th>{t.folder ? "Folder" : "Session and accounts"}</th>
              <th class="num">{showCost ? "At API prices" : "Tokens"}</th>
              <th class="act"><span class="visually-hidden">Open</span></th>
            </tr>
          </thead>
          <tbody>
            {#each t.report.rows.slice(0, 8) as r (r.key)}
              <tr>
                <td class="truncate" title={r.key ?? ""}>{t.folder ? r.key : `${(r.key ?? "").slice(0, 8)}  ${r.accounts.map(name).join(", ")}`}</td>
                <td class="num">{showCost ? money(r.cost) : shortTokens(r.tokens)}</td>
                <td class="act">
                  {#if t.folder && r.key}
                    {@const folder = r.key}
                    <button
                      class="plain icon-btn"
                      aria-label="Open a terminal in {folder}"
                      title="Open a terminal here"
                      onclick={() => actions.terminal({ kind: "shell" }, folder)}><Icon name="terminal" /></button
                    >
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    {/each}

    <div class="row footer">
      <label for="keep">Keep history for</label>
      <select id="keep" value={String(status?.keepDays ?? 90)} onchange={(e) => keepDays(e.currentTarget.value)}>
        <option value="30">30 days</option>
        <option value="90">90 days</option>
        <option value="365">1 year</option>
        <option value="0">Forever</option>
      </select>
      {#if store.prefs}
        <label class="check">
          <input
            type="checkbox"
            role="switch"
            class="switch"
            checked={store.prefs.weeklySummary}
            onchange={(e) => store.savePrefs({ weeklySummary: e.currentTarget.checked })}
          />
          Weekly summary notification
        </label>
      {/if}
    </div>
    <p class="caption tertiary">
      Costs are what the tokens would cost at Anthropic's API prices as of {byAccount.pricesAsOf}; plans are
      flat-priced, so this is the value of your usage, not a bill. ~ marks days whose account was worked out
      from the switch log. No conversation text is stored. {usageKept(haryxAccount.usageSyncs)}
    </p>
  {/if}
{/if}

<style>
  .divider {
    border: none;
    border-top: 1px solid var(--separator);
    margin: 24px 0 0;
  }
  h2.section {
    margin: 18px 0 10px;
    font-size: 16px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .choice {
    max-width: 560px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h2 {
    margin: 18px 0 6px;
    font-size: 14px;
    font-weight: 600;
  }
  .stats {
    display: flex;
    gap: 28px;
    margin-top: 12px;
  }
  .stats div {
    display: flex;
    flex-direction: column;
  }
  .stats strong {
    font-size: 20px;
  }
  .chart {
    width: 100%;
    max-width: 960px;
    height: auto;
  }
  .grid {
    stroke: var(--separator);
  }
  .tick,
  .axis {
    font-size: 11px;
    fill: var(--label-2);
  }
  .tick {
    text-anchor: middle;
  }
  .axis {
    text-anchor: end;
  }
  .swatch {
    display: inline-block;
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-right: 6px;
    vertical-align: 1px;
  }
  th {
    text-align: left;
    font-weight: 400;
    padding: 4px 8px;
  }
  .icon-btn {
    display: inline-flex;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
  }
  .banner span {
    flex: 1;
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
  }
  td {
    padding: 4px 8px;
    border-top: 1px solid var(--separator);
  }
  .num {
    text-align: right;
    width: 110px;
  }
  .act {
    width: 36px;
  }
  .footer {
    margin-top: 16px;
  }
</style>
