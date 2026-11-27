<script lang="ts">
  // Names a group and ticks the accounts that go in it. Opened from the Accounts pane's New Group
  // button and from a card's right-click menu (with that account ticked). Mirrors macOS NewGroupSheet.
  import { onMount } from "svelte";
  import { actions } from "../lib/actions";
  import { GROUP_MAX_CHARS, accountGroups, groupLabel, groupNames, normalizeGroupName } from "../lib/groups";
  import { store } from "../lib/store.svelte";
  import { displayName } from "../lib/tide";

  let { initial, onclose }: { initial: number[]; onclose: () => void } = $props();

  let dialog: HTMLDialogElement;
  let name = $state("");
  // The dialog starts from the accounts it was opened with; later ticks are its own.
  // svelte-ignore state_referenced_locally
  let selection = $state<number[]>([...initial]);

  let normalized = $derived(normalizeGroupName(name, store.accounts));
  let joinsExisting = $derived(normalized != null && groupNames(store.accounts).includes(normalized));

  onMount(() => dialog.showModal());

  function toggle(n: number, on: boolean) {
    selection = on ? [...selection, n] : selection.filter((x) => x !== n);
  }

  async function create(e: SubmitEvent) {
    e.preventDefault();
    if (normalized == null || selection.length === 0) return;
    const members = store.accounts.filter((a) => selection.includes(a.number));
    if (await actions.addToGroupMany(members, normalized)) dialog.close();
  }
</script>

<dialog bind:this={dialog} aria-labelledby="new-group-title" onclose={onclose}>
  <form onsubmit={create}>
    <h2 id="new-group-title">New group</h2>
    <p class="caption secondary">
      An account can be in several groups. Automatic switching tries accounts that share a group with the active one first, and leaves them only when none can take over.
    </p>
    <label class="name">
      <span>Name</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={name} placeholder="e.g. Work" maxlength={GROUP_MAX_CHARS} autofocus />
    </label>
    {#if joinsExisting}<p class="caption secondary">Adds to the existing group {normalized}.</p>{/if}
    <fieldset>
      <legend>Accounts</legend>
      {#each store.accounts as a (a.number)}
        <label class="member">
          <input
            type="checkbox"
            checked={selection.includes(a.number)}
            onchange={(e) => toggle(a.number, e.currentTarget.checked)}
          />
          <span class="truncate">{displayName(a)}</span>
          <span class="caption tertiary">{accountGroups(a).length > 0 ? `also in ${groupLabel(a)}` : `Slot ${a.number}`}</span>
        </label>
      {/each}
    </fieldset>
    <div class="buttons">
      <button type="button" onclick={() => dialog.close()}>Cancel</button>
      <button type="submit" class="primary" disabled={normalized == null || selection.length === 0 || !!store.busy}>Create</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: min(440px, calc(100vw - 32px));
    padding: 20px;
    border: 1px solid var(--separator);
    border-radius: var(--radius-card);
    background: var(--surface);
    color: inherit;
  }
  dialog::backdrop {
    background: rgb(0 0 0 / 0.3);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h2 {
    margin: 0;
    font-size: 16px;
  }
  p {
    margin: 0;
  }
  .name {
    display: grid;
    grid-template-columns: 60px 1fr;
    align-items: center;
    gap: 8px;
  }
  fieldset {
    margin: 0;
    padding: 6px 8px;
    border: 1px solid var(--separator);
    border-radius: var(--radius-banner);
    max-height: 300px;
    overflow-y: auto;
  }
  legend {
    color: var(--label-2);
    padding: 0 4px;
  }
  .member {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 8px;
    padding: 4px 0;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
