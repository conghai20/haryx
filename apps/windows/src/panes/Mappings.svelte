<script lang="ts">
  // Folder mappings (macOS `MappingsPane`): folders whose plain `haryx ai run` uses a set account.
  // Lists them with that account's mascot, runs Claude in one, removes one, and maps a new folder.
  import { actions } from "../lib/actions";
  import { store } from "../lib/store.svelte";
  import { displayName } from "../lib/tide";
  import Mascot from "../components/Mascot.svelte";

  let newSlot = $state("");

  async function chooseAndMap() {
    const a = store.accounts.find((x) => String(x.number) === newSlot);
    if (!a) return;
    const folder = await actions.chooseFolder("Map");
    if (folder) await actions.map(a, folder);
  }
</script>

<section aria-labelledby="m-list">
  <h2 id="m-list">Mapped folders</h2>
  {#if store.mappings.length === 0}
    <p class="secondary">No folders mapped. Mapped folders run Claude as their account with plain <code>haryx ai run</code>.</p>
  {:else}
    <ul>
      {#each store.mappings as m (m.path)}
        {@const a = store.accountFor(m)}
        <li>
          {#if a}<Mascot account={a} size={28} />{:else}<span class="no-mascot"></span>{/if}
          <div class="grow">
            <div class="mono truncate" title={m.path}>{m.path}</div>
            <div class="caption secondary">{a ? `Account ${a.number}: ${displayName(a)}` : `${m.email} (not managed)`}</div>
          </div>
          <button onclick={() => actions.terminal({ kind: "run-mapped" }, m.path)}>Run Claude</button>
          <button class="mini danger" disabled={!!store.busy} aria-label="Unmap {m.path}" title="Unmap" onclick={() => actions.unmap(m.path)}>Unmap</button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<div class="hairline"></div>

<section aria-labelledby="m-new">
  <h2 id="m-new">Map a folder</h2>
  <div class="line">
    <label>Account
      <select bind:value={newSlot}>
        <option value="">Choose…</option>
        {#each store.accounts as a (a.number)}<option value={String(a.number)}>{a.number}: {displayName(a)}</option>{/each}
      </select>
    </label>
    <button disabled={newSlot === "" || !!store.busy} onclick={chooseAndMap}>Choose folder and map…</button>
  </div>
</section>

<style>
  h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  section p {
    margin: 0;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  li {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .no-mascot {
    width: 28px;
    flex: none;
  }
  .mono {
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .danger {
    color: var(--red);
  }
</style>
