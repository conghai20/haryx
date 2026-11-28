<script lang="ts">
  import { onMount } from "svelte";
  import State from "../components/State.svelte";
  import * as AlertDialog from "$lib/components/ui/alert-dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import CommandField from "../components/CommandField.svelte";
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
    if (e.key === "Enter") { e.preventDefault(); void save(m); }
    else if (e.key === "Escape") { e.stopPropagation(); editing = null; }
  }
  const osName = (os: string) => ({ macos: "macOS", windows: "Windows", linux: "Linux" })[os.toLowerCase()] ?? os;

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

{#if machines === null && !failed}
  <State kind="loading" />
{:else if failed}
  <State kind="error" title="Could not load your machines" onAction={load} />
{:else if machines!.length === 0}
  <State kind="empty" title="No machines to show" detail="A Mac or PC appears here once it is signed in to HaryX and “Show this machine on HaryX web” is on in its app. From a terminal:">
    <CommandField command="haryx account consent grant machines" />
  </State>
{:else}
  <ul class="border-rule max-w-4xl border-b">
    {#each machines! as m (m.machineId)}
      <li class="border-rule flex flex-wrap items-center justify-between gap-x-6 gap-y-3 border-t py-6">
        <div class="grid min-w-0 flex-[1_1_16rem] gap-1">
          {#if editing === m.machineId}
            <label class="sr-only" for={`n-${m.machineId}`}>Machine name</label>
            <form class="flex flex-wrap gap-2" onsubmit={(e) => { e.preventDefault(); void save(m); }}>
              <!-- svelte-ignore a11y_autofocus -->
              <Input id={`n-${m.machineId}`} type="text" class="max-w-xs" bind:value={draft} onkeydown={(e: KeyboardEvent) => onKey(e, m)} maxlength={80} autofocus aria-invalid={nameError ? "true" : undefined} />
              <Button type="submit">Save</Button>
              <Button variant="ghost" onclick={() => (editing = null)}>Cancel</Button>
            </form>
            {#if nameError}<p class="text-red-text text-sm" role="alert">{nameError}</p>{/if}
          {:else}
            <p class="flex flex-wrap items-baseline gap-x-3 text-[17px] font-semibold">
              <span class="break-words">{m.name}</span>
              {#if m.isThisMachine}<span class="text-ink-3 text-sm font-normal">This machine</span>{/if}
              {#if !m.signedIn}<span class="text-ink-3 text-sm font-normal">Signed out</span>{/if}
            </p>
          {/if}
          <p class="text-ink-3 text-sm">{osName(m.os)} · HaryX {m.version} · <time datetime={m.lastSeenAt} title={exactTime(m.lastSeenAt)}>seen {lastSeen(m.lastSeenAt)}</time></p>
        </div>
        {#if editing !== m.machineId}
          <div class="flex gap-2">
            <Button variant="ghost" onclick={() => startEdit(m)}>Rename</Button>
            {#if m.signedIn}<Button variant="destructive-outline" onclick={() => ask(m)}>Sign out</Button>{/if}
          </div>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

<AlertDialog.Root open={confirming !== null} onOpenChange={(open) => { if (!open && !busy) confirming = null; }}>
  <AlertDialog.Content>
    {#if confirming}
      <AlertDialog.Header>
        <AlertDialog.Title>Sign out {confirming.name}?</AlertDialog.Title>
        <AlertDialog.Description>
          {confirming.isThisMachine ? "This is the machine you are using. " : ""}It will be signed out of WeCoAp. Its next sync asks to sign in again; local data on that machine stays.
        </AlertDialog.Description>
      </AlertDialog.Header>
      {#if signOutError}<p class="text-red-text text-sm" role="alert">{signOutError}</p>{/if}
      <AlertDialog.Footer>
        <AlertDialog.Cancel disabled={busy}>Cancel</AlertDialog.Cancel>
        <Button variant="destructive" onclick={confirmSignOut} disabled={busy}>{busy ? "Signing out…" : sessionEnded ? "Retry" : "Sign out"}</Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
