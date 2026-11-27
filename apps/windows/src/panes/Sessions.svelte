<script lang="ts">
  // Running Claude Code sessions (from `haryx ai sessions --json`), newest first, with their account's
  // mascot; each can continue on another account. Below them, Claude as any account in a new terminal.
  import { actions } from "../lib/actions";
  import { store } from "../lib/store.svelte";
  import { middleTruncate, tildePath } from "../lib/text";
  import { displayName } from "../lib/tide";
  import ContinueMenu from "../components/ContinueMenu.svelte";
  import Icon from "../components/Icon.svelte";
  import Mascot from "../components/Mascot.svelte";

  /** Names past this length are cut in the middle: emails that share a start stay apart. */
  const NAME_MAX = 36;

  let stuckPids = $derived(new Set(store.stuck.map((s) => s.session.pid)));

  let sessions = $derived([...store.sessions.sessions].sort((a, b) => b.startedAt - a.startedAt));

  function nameFor(n: number | null, email: string): string {
    const account = store.accounts.find((a) => a.number === n);
    return middleTruncate(account ? displayName(account) : email || "no active account", NAME_MAX);
  }

  const accountOf = (n: number | null) => store.accounts.find((a) => a.number === n) ?? null;

  const fiveHour = (pct: number | null | undefined) => (pct == null ? null : `${Math.round(pct)}%`);

  function since(ms: number): string {
    const minutes = Math.max(0, Math.round((store.now - ms) / 60_000));
    if (minutes < 60) return `${minutes}m`;
    const hours = Math.floor(minutes / 60);
    return hours < 24 ? `${hours}h ${minutes % 60}m` : `${Math.floor(hours / 24)}d ${hours % 24}h`;
  }
</script>

<section aria-labelledby="running-title">
  <h2 id="running-title">Running</h2>
  {#if sessions.length === 0}
    <p class="secondary">No Claude Code sessions running.</p>
  {:else}
    <ul class="sessions">
      {#each sessions as s (s.pid)}
        {@const a = accountOf(s.account)}
        <li>
          <span class="mascot">{#if a}<Mascot account={a} size={28} />{/if}</span>
          <div class="who">
            <span class="name" title={s.email}>
              {nameFor(s.account, s.email)}
              {#if s.profile === "session"}<span class="badge" title="Started with haryx ai run: it keeps its own login">own login</span>{/if}
            </span>
            <span class="folder truncate" title={s.cwd}>{tildePath(s.cwd)}</span>
            <span class="caption">
              {#if stuckPids.has(s.pid)}<span class="stuck">Out of quota</span> ·
              {:else if s.status === "busy"}Working ·
              {:else if s.status}{s.status.charAt(0).toUpperCase()}{s.status.slice(1)} ·{/if}
              running for {since(s.startedAt)} · <span title="Process ID">PID {s.pid}</span>
            </span>
          </div>
          <ContinueMenu session={s} from={s.profile === "session" ? s.account : null} />
        </li>
      {/each}
    </ul>
    <p class="caption">Continue on… ends a session and resumes the same conversation on another account, in a new terminal.</p>
  {/if}
  {#if store.sessions.unreadable > 0}
    <p class="caption">
      {store.sessions.unreadable} session record{store.sessions.unreadable === 1 ? "" : "s"} could not be read; more may be running.
    </p>
  {/if}
</section>

<section aria-labelledby="start-title">
  <h2 id="start-title">Start Claude as an account in a new terminal</h2>
  <ul class="start">
    {#each store.accounts as a (a.number)}
      <li>
        <span class="grow truncate" title={a.email}>{a.number}. {middleTruncate(displayName(a), NAME_MAX)}</span>
        {#if fiveHour(a.usage?.fiveHour?.pct)}<span class="caption">5-hour {fiveHour(a.usage?.fiveHour?.pct)}</span>{/if}
        <button class="run" onclick={() => actions.runAs(a)} aria-label="Run Claude as {displayName(a)}">
          <Icon name="terminal" size={14} />Run Claude
        </button>
      </li>
    {/each}
  </ul>
</section>

<style>
  section {
    margin-bottom: 24px;
  }
  h2 {
    font-size: 13px;
    font-weight: 600;
    margin: 0 0 6px;
  }
  p {
    margin: 6px 0 0;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .sessions li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 0;
    border-top: 1px solid var(--separator);
  }
  .mascot {
    width: 28px;
    flex: none;
  }
  .who {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .name {
    font-weight: 600;
  }
  .folder {
    color: var(--label-2);
  }
  .badge {
    margin-left: 6px;
  }
  .stuck {
    color: var(--amber);
    font-weight: 600;
  }
  .start li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 0;
    border-top: 1px solid var(--separator);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .run {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
</style>
