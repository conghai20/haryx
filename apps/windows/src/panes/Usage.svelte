<script lang="ts">
  // AI › Usage: quota over time, tokens (or API-price cost) per day by account, and the projects
  // and sessions that used the most. From `haryx ai usage --json`; kept on this PC.
  import { api } from "../lib/api";
  import { actions } from "../lib/actions";
  import { store } from "../lib/store.svelte";
  import { displayName } from "../lib/tide";
  import {
    accountColor,
    money,
    quotaLine,
    shortTokens,
    stackedBars,
    type HistoryStatus,
    type QuotaReport,
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
  let error = $state<string | null>(null);
  let status = $state<HistoryStatus | null>(null);
  let byAccount = $state<UsageReport | null>(null);
  let byDay = $state<Record<number, UsageReport>>({});
  let projects = $state<UsageReport | null>(null);
  let sessions = $state<UsageReport | null>(null);
  let quota = $state<QuotaReport | null>(null);

  function name(n: number): string {
    const a = store.accounts.find((x) => x.number === n);
    return a ? displayName(a) : `Account ${n}`;
  }

  async function load() {
    loading = true;
    try {
      const base = ["--since", range, ...(account != null ? ["--account", String(account)] : [])];
      const first = await api.json<UsageReport>(["ai", "usage", "report", "--by", "account", ...base, "--json"]);
      status = first.history ?? null;
      if (!status?.enabled || status.needsChoice) {
        byAccount = null;
        return;
      }
      const numbers = first.rows.map((r) => Number(r.key)).filter((n) => Number.isFinite(n));
      const [p, s, q, ...days] = await Promise.all([
        api.json<UsageReport>(["ai", "usage", "report", "--by", "project", ...base, "--json"]),
        api.json<UsageReport>(["ai", "usage", "report", "--by", "session", ...base, "--json"]),
        api.json<QuotaReport>(["ai", "usage", "quota", ...base, "--json"]),
        ...numbers.map((n) =>
          api.json<UsageReport>(["ai", "usage", "report", "--by", "day", "--since", range, "--account", String(n), "--json"]),
        ),
      ]);
      byAccount = first;
      projects = p;
      sessions = s;
      quota = q;
      const perDay: Record<number, UsageReport> = {};
      numbers.forEach((n, i) => {
        const report = days[i];
        if (report) perDay[n] = report;
      });
      byDay = perDay;
      error = null;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function choose(all: boolean) {
    await api.haryx(["ai", "usage", "index", "--import", all ? "all" : "none", "--json"]);
    await load();
  }

  async function keepDays(days: string) {
    await api.haryx(["config", "set", "history.keepDays", days]);
    await load();
  }

  $effect(() => {
    void range;
    void account;
    load();
  });

  let bars = $derived(stackedBars(byDay, showCost, W, H));
  let accountsInQuota = $derived([...new Set((quota?.readings ?? []).map((r) => r.account))].sort((a, b) => a - b));
  let times = $derived((quota?.readings ?? []).map((r) => r.at));
  let from = $derived(times.length ? Math.min(...times) : 0);
  let to = $derived(times.length ? Math.max(...times) : 1);
</script>

{#if status?.needsChoice}
  <section class="choice">
    <h2>Start your usage history</h2>
    <p class="secondary">
      HaryX can read your past Claude Code transcripts to show how each account, project and session used
      its quota. It reads token counts only, never the conversations, and keeps them on this PC.
    </p>
    <div class="row">
      <button class="primary" onclick={() => choose(true)}>Read past transcripts</button>
      <button onclick={() => choose(false)}>Start from now</button>
    </div>
  </section>
{:else if status && !status.enabled}
  <p class="secondary">Usage history is off. Turn it on in Settings › haryx settings (history.enabled).</p>
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
    {#if loading}<span class="caption secondary">Reading…</span>{/if}
    <button class="plain" title="Read new transcript lines and refresh" onclick={load}>⟳</button>
  </div>
  {#if error}<div class="banner error">{error}</div>{/if}

  {#if byAccount}
    <div class="stats">
      <div><strong>{shortTokens(byAccount.total.tokens)}</strong><span class="caption secondary">Tokens</span></div>
      <div><strong>{money(byAccount.total.cost)}</strong><span class="caption secondary">At API prices</span></div>
      <div><strong>{byAccount.total.replies}</strong><span class="caption secondary">Replies</span></div>
      <div><strong>{shortTokens(byAccount.total.output)}</strong><span class="caption secondary">Output tokens</span></div>
    </div>

    <h2>Quota over time</h2>
    {#if (quota?.readings ?? []).length === 0}
      <p class="caption secondary">Readings appear here as HaryX reads usage.</p>
    {:else}
      <svg viewBox="0 0 {W} {H}" class="chart" role="img" aria-label="5-hour and weekly usage per account over time">
        <line x1="0" x2={W} y1={H - (100 / 105) * H} y2={H - (100 / 105) * H} class="limit" />
        {#each accountsInQuota as n (n)}
          <polyline points={quotaLine(quota?.readings ?? [], n, "fiveHour", from, to, W, H)} stroke={accountColor(n)} fill="none" stroke-width="1.5" />
          <polyline points={quotaLine(quota?.readings ?? [], n, "sevenDay", from, to, W, H)} stroke={accountColor(n)} fill="none" stroke-width="1.5" stroke-dasharray="4 3" />
        {/each}
      </svg>
      <p class="caption tertiary legend">
        Solid: 5-hour window · dashed: weekly ·
        {#each accountsInQuota as n (n)}<span style="color: {accountColor(n)}">■ {name(n)}</span> {/each}
      </p>
    {/if}

    <h2>{showCost ? "API cost per day" : "Tokens per day"}</h2>
    {#if bars.bars.length === 0}
      <p class="caption secondary">No usage in this range.</p>
    {:else}
      <svg viewBox="0 0 {W} {H + 16}" class="chart" role="img" aria-label="Usage per day by account">
        {#each bars.bars as b (b.day)}
          {#each b.segments as s (s.account)}
            <rect x={b.x} y={s.y} width={b.width} height={s.height} fill={accountColor(s.account)}>
              <title>{b.day} · {name(s.account)} · {showCost ? money(s.value) : shortTokens(s.value)}</title>
            </rect>
          {/each}
          {#if bars.bars.length <= 31}<text x={b.x + b.width / 2} y={H + 12} class="tick">{b.day.slice(5)}</text>{/if}
        {/each}
      </svg>
    {/if}

    <h2>By account</h2>
    <table>
      <tbody>
        {#each byAccount.rows as r (r.key)}
          <tr>
            <td class="truncate">{r.key && r.key !== "unknown" ? name(Number(r.key)) : "Unknown account"}{#if r.approximate} <span class="tertiary" title="Account worked out from the switch log, so approximate">~</span>{/if}</td>
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
          <tbody>
            {#each t.report.rows.slice(0, 8) as r (r.key)}
              <tr>
                <td class="truncate" title={r.key ?? ""}>{t.folder ? r.key : `${(r.key ?? "").slice(0, 8)}  ${r.accounts.map(name).join(", ")}`}</td>
                <td class="num">{showCost ? money(r.cost) : shortTokens(r.tokens)}</td>
                <td class="act">
                  {#if t.folder && r.key}<button class="plain" title="Open a terminal here" onclick={() => actions.terminal({ kind: "shell" }, r.key)}>›_</button>{/if}
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
          <input type="checkbox" checked={store.prefs.weeklySummary} onchange={(e) => store.savePrefs({ weeklySummary: e.currentTarget.checked })} />
          Weekly summary notification
        </label>
      {/if}
    </div>
    <p class="caption tertiary">
      Costs are what the tokens would cost at Anthropic's API prices as of {byAccount.pricesAsOf}; plans are
      flat-priced, so this is the value of your usage, not a bill. ~ marks days whose account was worked out
      from the switch log. No conversation text is stored.
    </p>
  {/if}
{/if}

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
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
    max-width: 720px;
    height: auto;
  }
  .limit {
    stroke: #d64545;
    stroke-opacity: 0.5;
  }
  .tick {
    font-size: 9px;
    text-anchor: middle;
    fill: var(--label-2);
  }
  .legend span {
    margin-right: 6px;
  }
  table {
    width: 100%;
    max-width: 720px;
    border-collapse: collapse;
    table-layout: fixed;
  }
  td {
    padding: 4px 8px;
    border-top: 1px solid var(--separator);
  }
  .num {
    text-align: right;
    width: 90px;
  }
  .act {
    width: 36px;
  }
  .footer {
    margin-top: 16px;
  }
</style>
