<script lang="ts">
  // The Accounts list over unlocked data: identity and settings from `account` rows, the mascot from
  // `app-prefs`, and the machines whose synced usage names each account.
  import { onMount } from "svelte";
  import State from "./State.svelte";
  import Mascot from "./Mascot.svelte";
  import { api } from "../lib/auth.svelte";
  import { isGranted } from "../lib/consents";
  import { labelMachines } from "../lib/machinenames";
  import { machinesByAccount, normEmail } from "../lib/usagestats";
  import { resolve } from "@win/lib/mascot";
  import type { Synced } from "../lib/synced";
  import type { Consent, HaryxMachine } from "../lib/types";

  let { synced, consents }: { synced: Synced; consents: Consent[] } = $props();

  // Machine names are a nicety: without them (consent off, or the list failed) ids stand in.
  let registry = $state.raw<HaryxMachine[]>([]);
  let registryFailed = $state(false);
  onMount(() => {
    if (!isGranted(consents, "machines")) return;
    api<{ machines: HaryxMachine[] }>("/machines").then(
      (r) => (registry = r.machines),
      () => (registryFailed = true),
    );
  });

  const idOf = (email: string, org: string) => `${email}|${org}`;
  const styles = $derived(resolve(synced.mascotStyles, synced.accounts.map((a) => idOf(a.email, a.organizationUuid))));
  const used = $derived(machinesByAccount(synced.usage));
</script>

{#if synced.accounts.length === 0}
  <State kind="empty" title="No synced accounts yet" detail="Accounts appear here after a machine with Sync turned on runs a sync." />
{:else}
  <ul>
    {#each synced.accounts as a (idOf(a.email, a.organizationUuid))}
      {@const machines = labelMachines(used.get(normEmail(a.email) ?? "") ?? [], registry)}
      <li class:off={a.disabled}>
        <Mascot style={styles[idOf(a.email, a.organizationUuid)]!} />
        <div class="info">
          <strong>{a.alias ?? a.email}</strong>
          {#if a.alias}<span class="email">{a.email}</span>{/if}
          <span class="meta">{[a.organizationName, a.priority !== null ? `Priority ${a.priority}` : "No priority set", a.disabled ? "Off: held out of rotation" : ""].filter(Boolean).join(" · ")}</span>
          <div class="machines">
            <span class="label">Used on</span>
            {#each machines as m (m.id)}
              <span class="tag" class:dim={m.signedIn === false} title={m.signedIn === false ? "Signed out of HaryX" : m.id}>{m.name}{m.isThisMachine ? " (this one)" : ""}{m.signedIn === false ? " · signed out" : ""}</span>
            {:else}
              <span class="none">No synced usage yet</span>
            {/each}
          </div>
        </div>
      </li>
    {/each}
  </ul>
  {#if !isGranted(consents, "machines")}
    <p class="note">Machine names show when Machines is on in the <a href="#privacy">Privacy center</a>.</p>
  {:else if registryFailed}
    <p class="note">Could not load machine names; showing machine ids.</p>
  {/if}
  <p class="note">“Used on” lists the machines whose synced usage names the account. Sign-in tokens are synced encrypted and never shown here.</p>
{/if}

<style>
  ul { list-style: none; margin: 0; padding: 0; display: grid; gap: .6rem; max-width: 52rem; }
  li { background: var(--surface); border: 1px solid var(--separator); border-radius: var(--radius-card); padding: .8rem 1rem; display: flex; gap: .9rem; align-items: flex-start; }
  li.off { opacity: .72; }
  .info { display: grid; gap: .15rem; min-width: 0; flex: 1; }
  strong, .email { overflow-wrap: anywhere; }
  .email, .meta, .note { color: var(--label-2); font-size: .85rem; }
  .machines { display: flex; flex-wrap: wrap; gap: .3rem; align-items: center; margin-top: .35rem; }
  .label { font-size: .75rem; color: var(--label-2); margin-right: .15rem; }
  .tag { font-size: .75rem; background: var(--selected); border-radius: 99px; padding: .05rem .5rem; overflow-wrap: anywhere; }
  .tag.dim { background: var(--fill-strong); }
  .none { font-size: .75rem; color: var(--label-2); }
  .note { max-width: 52rem; margin-top: .75rem; }
</style>
