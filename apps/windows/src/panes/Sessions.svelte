<script lang="ts">
  // Running Claude Code sessions (from `haryx ai sessions --json`), newest first, with their account.
  import { actions } from "../lib/actions";
  import { store } from "../lib/store.svelte";
  import { displayName } from "../lib/tide";
  import { bestTarget, peakPct } from "../lib/handoff";
  import type { Session } from "../lib/types";

  let stuckPids = $derived(new Set(store.stuck.map((s) => s.session.pid)));

  function continueOn(s: Session, value: string) {
    if (!value) return;
    actions.continueSession(s.pid, value === "best" ? null : Number(value), s.cwd);
  }

  function choices(s: Session) {
    const from = s.profile === "session" ? s.account : null;
    return store.accounts.filter((a) => a.number !== from && !a.disabled);
  }

  let sessions = $derived([...store.sessions.sessions].sort((a, b) => b.startedAt - a.startedAt));

  function nameFor(n: number | null, email: string): string {
    const account = store.accounts.find((a) => a.number === n);
    return account ? displayName(account) : email || "no active account";
  }

  function since(ms: number): string {
    const minutes = Math.max(0, Math.round((store.now - ms) / 60_000));
    if (minutes < 60) return `${minutes}m`;
    const hours = Math.floor(minutes / 60);
    return hours < 24 ? `${hours}h ${minutes % 60}m` : `${Math.floor(hours / 24)}d ${hours % 24}h`;
  }
</script>

{#if sessions.length === 0}
  <p class="secondary">No Claude Code sessions running.</p>
{:else}
  <table>
    <thead>
      <tr class="caption tertiary">
        <th>Account</th><th>Folder</th><th>Status</th><th class="num">Running for</th><th class="num">PID</th><th class="act"></th>
      </tr>
    </thead>
    <tbody>
      {#each sessions as s (s.pid)}
        <tr>
          <td class="truncate" title={s.email}>
            {nameFor(s.account, s.email)}
            {#if s.profile === "session"}<span class="badge">own profile</span>{/if}
          </td>
          <td class="truncate folder" title={s.cwd}>{s.cwd}</td>
          <td>
            {#if stuckPids.has(s.pid)}<span class="stuck">out of quota</span>
            {:else if s.status === "busy"}<span class="dot"></span> working{:else}{s.status ?? "–"}{/if}
          </td>
          <td class="num">{since(s.startedAt)}</td>
          <td class="num tertiary">{s.pid}</td>
          <td class="act">
            <select
              aria-label="Continue this session on another account"
              title="Ends this session and resumes the same conversation on another account, in a new terminal"
              value=""
              onchange={(e) => {
                continueOn(s, e.currentTarget.value);
                e.currentTarget.value = "";
              }}
            >
              <option value="">Continue on…</option>
              {#if bestTarget(store.accounts, s.profile === "session" ? s.account : null)}
                <option value="best">Best account</option>
              {/if}
              {#each choices(s) as a (a.number)}
                <option value={String(a.number)}>{a.number}. {displayName(a)}{peakPct(a) != null ? ` (${peakPct(a)}%)` : ""}</option>
              {/each}
            </select>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}
{#if store.sessions.unreadable > 0}
  <p class="caption secondary">
    {store.sessions.unreadable} session record(s) could not be read; more may be running.
  </p>
{/if}

<div class="hairline"></div>
<p class="caption secondary">Start Claude as an account in a new terminal:</p>
<div class="start">
  {#each store.accounts as a (a.number)}
    <button onclick={() => actions.runAs(a)}>{displayName(a)}</button>
  {/each}
</div>

<style>
  table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
  }
  th {
    text-align: left;
    font-weight: 400;
    padding: 4px 8px;
  }
  td {
    padding: 6px 8px;
    border-top: 1px solid var(--separator);
  }
  .act {
    width: 150px;
  }
  .act select {
    width: 100%;
  }
  .stuck {
    color: var(--orange, #c75000);
    font-weight: 600;
  }
  .num {
    text-align: right;
    width: 90px;
  }
  .folder {
    width: 40%;
  }
  .badge {
    margin-left: 6px;
  }
  .dot {
    margin-right: 4px;
  }
  .start {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
</style>
