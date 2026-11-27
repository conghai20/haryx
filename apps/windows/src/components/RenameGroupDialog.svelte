<script lang="ts">
  // Renames a group on every account in it. A name that matches another group merges the two.
  // Mirrors macOS RenameGroupSheet.
  import { onMount } from "svelte";
  import { actions } from "../lib/actions";
  import { GROUP_MAX_CHARS, groupNames, isInGroup } from "../lib/groups";
  import { store } from "../lib/store.svelte";

  let { name, onclose }: { name: string; onclose: () => void } = $props();

  let dialog: HTMLDialogElement;
  // The field starts from the group's current name; later typing is its own.
  // svelte-ignore state_referenced_locally
  let value = $state(name);

  let trimmed = $derived(value.trim());
  let members = $derived(store.accounts.filter((a) => isInGroup(a, name)).length);
  let mergesInto = $derived(
    groupNames(store.accounts).find((g) => g.toLowerCase() === trimmed.toLowerCase() && g.toLowerCase() !== name.toLowerCase()) ??
      null,
  );
  let canRename = $derived(trimmed.length > 0 && [...trimmed].length <= GROUP_MAX_CHARS && trimmed !== name && !store.busy);

  onMount(() => dialog.showModal());

  async function rename(e: SubmitEvent) {
    e.preventDefault();
    if (!canRename) return;
    if (await actions.renameGroup(name, trimmed)) dialog.close();
  }
</script>

<dialog bind:this={dialog} aria-labelledby="rename-group-title" onclose={onclose}>
  <form onsubmit={rename}>
    <h2 id="rename-group-title">Rename {name}</h2>
    <p class="caption secondary">Renames the group on its {members} account{members === 1 ? "" : "s"}.</p>
    <label class="name">
      <span>Name</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value maxlength={GROUP_MAX_CHARS} autofocus />
    </label>
    {#if mergesInto}<p class="caption merge">Merges into the existing group {mergesInto}.</p>{/if}
    <div class="buttons">
      <button type="button" onclick={() => dialog.close()}>Cancel</button>
      <button type="submit" class="primary" disabled={!canRename}>{mergesInto ? "Merge" : "Rename"}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: min(400px, calc(100vw - 32px));
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
  .merge {
    color: var(--amber);
  }
  .name {
    display: grid;
    grid-template-columns: 60px 1fr;
    align-items: center;
    gap: 8px;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
