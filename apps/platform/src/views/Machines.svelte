<script lang="ts">
  import { onMount } from "svelte";
  import Section from "../components/Section.svelte";
  import State from "../components/State.svelte";
  import Dialog from "../components/Dialog.svelte";
  import { api } from "../lib/auth.svelte";
  import { ApiError } from "../lib/api";
  import { endSession } from "../lib/keycloak";
  import { exactTime, lastSeen, signOutMachine, signOutMessage, validateName } from "../lib/machines";
  import type { HaryxMachine } from "../lib/types";

  // The API sends each machine's `sessionId`, which names its Keycloak session for step 1 of
  // sign-out (A7). Without one (an older API), sign-out is refused with a message, never guessed.
  type Machine = HaryxMachine;

  let machines = $state<Machine[] | null>(null);
  let failed = $state(false);
  let editing = $state<string | null>(null);
  let draft = $state("");
  let nameError = $state("");
  let confirming = $state<Machine | null>(null);
  let busy = $state(false);
  let signOutError = $state("");
  let sessionEnded = $state(false);

  async function load() {
    failed = false;
    machines = null;
    try {
      machines = (await api<{ machines: Machine[] }>("/machines")).machines;
    } catch (e) {
      if (!(e instanceof ApiError && e.kind === "unauthorized")) failed = true;
    }
  }
  onMount(load);

  const patch = (m: Machine, next: Partial<Machine>) => (machines = machines!.map((x) => (x.machineId === m.machineId ? { ...x, ...next } : x)));

  function startEdit(m: Machine) { editing = m.machineId; draft = m.name; nameError = ""; }
  async function save(m: Machine) {
    const v = validateName(draft);
    if (!v.ok) { nameError = v.error; return; }
    if (v.name === m.name) { editing = null; return; }
    try {
      await api(`/machines/${m.machineId}`, { method: "PATCH", body: JSON.stringify({ name: v.name }) });
      patch(m, { name: v.name });
      editing = null;
    } catch {
      nameError = "Could not rename. Try again.";
    }
  }
  function onKey(e: KeyboardEvent, m: Machine) {
    if (e.key === "Enter") void save(m);
    else if (e.key === "Escape") { e.stopPropagation(); editing = null; }
  }

  function ask(m: Machine) { confirming = m; signOutError = ""; sessionEnded = false; }
  async function confirmSignOut() {
    const m = confirming!;
    busy = true;
    signOutError = "";
    if (!m.sessionId && !sessionEnded) {
      signOutError = "This machine's WeCoAp session can't be identified yet, so it can't be signed out from here.";
      busy = false;
      return;
    }
    const r = await signOutMachine(
      { endSession: () => endSession(m.sessionId!), record: () => api(`/machines/${m.machineId}/session`, { method: "DELETE" }) },
      { skipKeycloak: sessionEnded },
    );
    busy = false;
    if (r.ok) { patch(m, { signedIn: false }); confirming = null; return; }
    sessionEnded = r.failedAt === "haryx";
    signOutError = signOutMessage(r.failedAt);
  }
</script>

<Section title="Machines" blurb="Where HaryX is signed in.">
  {#if machines === null && !failed}
    <State kind="loading" />
  {:else if failed}
    <State kind="error" title="Could not load your machines" onAction={load} />
  {:else if machines!.length === 0}
    <State kind="empty" title="No machines to show" detail="A machine appears here once it is signed in to HaryX and “Show this machine on HaryX web” is on in its app, or you ran haryx account consent grant machines." />
  {:else}
    <ul>
      {#each machines! as m (m.machineId)}
        <li>
          <div class="info">
            {#if editing === m.machineId}
              <label class="sr" for={`n-${m.machineId}`}>Machine name</label>
              <!-- svelte-ignore a11y_autofocus -->
              <input id={`n-${m.machineId}`} type="text" bind:value={draft} onkeydown={(e) => onKey(e, m)} onblur={() => (editing = null)} maxlength="80" autofocus aria-invalid={nameError ? "true" : undefined} />
              {#if nameError}<span class="err" role="alert">{nameError}</span>{/if}
            {:else}
              <strong>{m.name}</strong>
              {#if m.isThisMachine}<span class="tag">This is you</span>{/if}
              {#if !m.signedIn}<span class="tag off">Signed out</span>{/if}
            {/if}
            <span class="meta">{m.os} · v{m.version} · <time datetime={m.lastSeenAt} title={exactTime(m.lastSeenAt)}>seen {lastSeen(m.lastSeenAt)}</time></span>
          </div>
          <div class="actions">
            <button onmousedown={(e) => e.preventDefault()} onclick={() => startEdit(m)}>Rename</button>
            <button class="danger" onclick={() => ask(m)} disabled={!m.signedIn}>Sign out</button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</Section>

{#if confirming}
  <Dialog title={`Sign out ${confirming.name}?`} onClose={() => (confirming = null)}>
    <p>It will be signed out of WeCoAp. Its next sync will ask to sign in again; local data on that machine stays.</p>
    {#if signOutError}<p class="err" role="alert">{signOutError}</p>{/if}
    <div class="row">
      <button onclick={() => (confirming = null)} disabled={busy}>Cancel</button>
      <button class="primary" onclick={confirmSignOut} disabled={busy}>{sessionEnded ? "Retry" : "Sign out"}</button>
    </div>
  </Dialog>
{/if}

<style>
  ul { list-style: none; margin: 0; padding: 0; display: grid; gap: .6rem; max-width: 44rem; }
  li { background: var(--surface); border: 1px solid var(--separator); border-radius: var(--radius-card); padding: .8rem 1rem; display: flex; gap: .75rem; align-items: center; justify-content: space-between; flex-wrap: wrap; min-height: 4.4rem; }
  .info { display: grid; gap: .15rem; min-width: 0; flex: 1 1 14rem; }
  strong { overflow-wrap: anywhere; margin-right: .4rem; }
  .meta { color: var(--label-2); font-size: .85rem; }
  .tag { font-size: .75rem; background: var(--selected); border-radius: 99px; padding: .05rem .5rem; margin-right: .3rem; }
  .tag.off { background: var(--fill-strong); }
  .actions, .row { display: flex; gap: .5rem; flex-wrap: wrap; }
  .row { justify-content: flex-end; margin-top: 1rem; }
  .err { color: var(--red); font-size: .85rem; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
</style>
