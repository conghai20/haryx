<script lang="ts">
  // Maintenance: export and import accounts, cswap and usage imports, unclaimed credentials, upgrade,
  // and purge. Destructive actions ask first; purge also answers the CLI's own y/N prompt.
  import { onMount } from "svelte";
  import { ask, open, save } from "@tauri-apps/plugin-dialog";
  import { api } from "../lib/api";
  import { store } from "../lib/store.svelte";
  import { displayName } from "../lib/tide";

  interface Unclaimed {
    id: string;
    slot: string;
    reason: string;
  }

  const JSON_FILTER = [{ name: "JSON", extensions: ["json"] }];
  const UNCLAIMED_RE = /^(\S+)\s+slot\s+(\S+)\s+(.*)$/;
  const HOLD_MAX_S = 86_400;
  const PURGE_WORD = "PURGE";

  let exportSlot = $state<string>("");
  let exportFull = $state(false);
  let importForce = $state(false);
  let holdSeconds = $state(0);
  let unclaimed = $state<Unclaimed[] | null>(null);
  let unclaimedError = $state<string | null>(null);
  let purgeText = $state("");

  const today = () => new Date().toISOString().slice(0, 10);
  const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

  async function loadUnclaimed() {
    try {
      const r = await api.haryx(["unclaimed"]);
      if (!r.ok) throw r.message;
      unclaimed = r.stdout
        .split(/\r?\n/)
        .map((line) => UNCLAIMED_RE.exec(line.trim()))
        .filter((m): m is RegExpExecArray => m != null)
        .map((m) => ({ id: m[1] ?? "", slot: m[2] ?? "", reason: m[3] ?? "" }));
      unclaimedError = null;
    } catch (e) {
      unclaimedError = String(e);
    }
  }

  async function exportAccounts() {
    try {
      const path = await save({ defaultPath: `haryx-export-${today()}.json`, filters: JSON_FILTER });
      if (!path) return;
      const args = ["export", path, ...(exportSlot ? ["--account", exportSlot] : []), ...(exportFull ? ["--full"] : [])];
      await store.run("Exporting", args, { success: `Exported to ${fileName(path)}` });
    } catch (e) {
      store.flash(String(e), true);
    }
  }

  async function pickJson(): Promise<string | null> {
    const path = await open({ multiple: false, directory: false, filters: JSON_FILTER });
    return typeof path === "string" ? path : null;
  }

  async function importAccounts() {
    try {
      const path = await pickJson();
      if (path) await store.run("Importing", ["import", path, ...(importForce ? ["--force"] : [])]);
    } catch (e) {
      store.flash(String(e), true);
    }
  }

  async function importUsage() {
    try {
      const path = await pickJson();
      if (!path) return;
      const hold = Math.max(0, Math.min(HOLD_MAX_S, Math.round(holdSeconds || 0)));
      await store.run("Importing usage", ["import-usage", path, ...(hold > 0 ? ["--hold", String(hold)] : [])]);
    } catch (e) {
      store.flash(String(e), true);
    }
  }

  const importCswap = () =>
    store.run("Importing from cswap", ["import", "--from-cswap"], { success: "Accounts imported from cswap" });

  async function purgeEntry(u: Unclaimed) {
    const ok = await ask(`Delete the stashed credential ${u.id} (slot ${u.slot})? Recovery is signing in again.`, {
      title: "Purge credential",
      kind: "warning",
      okLabel: "Purge",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    await store.run("Purging", ["unclaimed", "--purge", u.id]);
    await loadUnclaimed();
  }

  const upgrade = () => store.run("Upgrading haryx", ["upgrade"]);

  /** The data folder, or haryx.log selected in it, in Explorer. */
  const reveal = (what: "store" | "log") => api.reveal(what).catch((e) => store.flash(String(e), true));

  async function purgeAll() {
    const ok = await ask(
      "Every stored account, backup and setting is removed. Your current Claude Code login stays as it is. This can't be undone.",
      { title: "Purge all account data?", kind: "warning", okLabel: "Purge everything", cancelLabel: "Cancel" },
    );
    purgeText = "";
    if (!ok) return;
    try {
      if (store.autoRunning) await store.toggleAuto();
    } catch (e) {
      store.flash(String(e), true);
      return;
    }
    await store.run("Purging all data", ["purge"], { stdin: "y\n", success: "All account data removed" });
    await loadUnclaimed();
  }

  onMount(loadUnclaimed);
</script>

<section aria-labelledby="t-export">
  <h2 id="t-export">Export</h2>
  <div class="line">
    <label>Accounts
      <select bind:value={exportSlot}>
        <option value="">All accounts</option>
        {#each store.accounts as a (a.number)}<option value={String(a.number)}>{a.number}: {displayName(a)}</option>{/each}
      </select>
    </label>
  </div>
  <label class="line"><input type="checkbox" bind:checked={exportFull} /> Full export (whole .claude.json and credentials, same-machine backup)</label>
  <div><button disabled={!!store.busy} onclick={exportAccounts}>Export…</button></div>
  <p class="caption">Exports are plain JSON that includes login tokens. Keep the file somewhere safe or encrypt it.</p>
</section>

<div class="hairline"></div>

<section aria-labelledby="t-import">
  <h2 id="t-import">Import</h2>
  <label class="line"><input type="checkbox" bind:checked={importForce} /> Overwrite existing accounts</label>
  <div class="line">
    <button disabled={!!store.busy} onclick={importAccounts}>Import accounts…</button>
    <button disabled={!!store.busy} onclick={importCswap}>Import from cswap</button>
  </div>
  <div class="line">
    <label>Hold imported usage for
      <input type="number" min="0" max={HOLD_MAX_S} step="300" bind:value={holdSeconds} class="num" /> s
    </label>
    <button disabled={!!store.busy} onclick={importUsage}>Import usage from another machine…</button>
  </div>
  <p class="caption">Usage import reads another machine's <code>haryx ai list --json</code> output so both don't poll the usage endpoint.</p>
</section>

<div class="hairline"></div>

<section aria-labelledby="t-unclaimed">
  <div class="line">
    <h2 id="t-unclaimed">Unclaimed credentials</h2>
    <button class="plain" aria-label="Refresh unclaimed credentials" title="Refresh" onclick={loadUnclaimed}>⟳</button>
  </div>
  {#if unclaimedError}
    <div class="banner error" role="alert">{unclaimedError}</div>
  {:else if unclaimed == null}
    <p class="secondary" role="status">Loading…</p>
  {:else if unclaimed.length === 0}
    <p class="secondary">None</p>
  {:else}
    <ul>
      {#each unclaimed as u (u.id)}
        <li>
          <div class="grow">
            <div class="mono truncate">{u.id}</div>
            <div class="caption">slot {u.slot} · {u.reason}</div>
          </div>
          <button class="mini" disabled={!!store.busy} aria-label="Purge {u.id}" onclick={() => purgeEntry(u)}>Purge</button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<div class="hairline"></div>

<section aria-labelledby="t-haryx">
  <h2 id="t-haryx">haryx</h2>
  <p class="caption truncate" title={store.cliPath ?? ""}>Executable: {store.cliPath ?? "not found"}</p>
  {#if store.storeRoot}<p class="caption truncate" title={store.storeRoot}>Data folder: {store.storeRoot}</p>{/if}
  <div class="line">
    <button disabled={!!store.busy || !store.cliPath} onclick={upgrade}>Upgrade haryx</button>
    <button onclick={() => reveal("store")}>Open data folder</button>
    <button onclick={() => reveal("log")}>Show log</button>
  </div>
</section>

<div class="hairline"></div>

<section aria-labelledby="t-purge">
  <h2 id="t-purge">Purge all account data</h2>
  <p class="caption">Removes every stored account, backup and setting. Your current Claude Code login stays as it is.</p>
  <div class="line">
    <label>Type {PURGE_WORD} to confirm <input bind:value={purgeText} placeholder={PURGE_WORD} autocomplete="off" /></label>
    <button class="danger" disabled={purgeText !== PURGE_WORD || !!store.busy} onclick={purgeAll}>Purge…</button>
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
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .num {
    width: 80px;
  }
  .mono,
  code {
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 0;
    border-top: 1px solid var(--separator);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .danger:not(:disabled) {
    color: var(--red);
    font-weight: 600;
  }
</style>
