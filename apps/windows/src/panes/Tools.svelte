<script lang="ts">
  // Backup & maintenance: export and import accounts, cswap and usage imports, unclaimed credentials,
  // the CLI, and (last, on its own) erasing everything. Destructive actions ask first; purge also
  // answers the CLI's own y/N prompt.
  import { onMount } from "svelte";
  import { ask, open, save } from "@tauri-apps/plugin-dialog";
  import { api } from "../lib/api";
  import { describeError } from "../lib/errors";
  import { store } from "../lib/store.svelte";
  import { middleTruncate } from "../lib/text";
  import { displayName } from "../lib/tide";
  import Icon from "../components/Icon.svelte";

  interface Unclaimed {
    id: string;
    slot: string;
    reason: string;
  }

  const JSON_FILTER = [{ name: "JSON", extensions: ["json"] }];
  const UNCLAIMED_RE = /^(\S+)\s+slot\s+(\S+)\s+(.*)$/;
  const HOLD_MAX_S = 86_400;
  const HOLD_STEP_S = 300;
  /** Export rows: long emails are cut in the middle so shared starts stay apart. */
  const NAME_MAX = 44;
  const PURGE_WORD = "PURGE";

  /** Accounts ticked for export; all of them to start with. */
  let exportSlots = $state<number[] | null>(null);
  let chosen = $derived(exportSlots ?? store.accounts.map((a) => a.number));
  let exportFull = $state(false);
  let importForce = $state(false);
  let holdSeconds = $state(0);
  let unclaimed = $state<Unclaimed[] | null>(null);
  let unclaimedError = $state<{ text: string; detail: string } | null>(null);
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
      unclaimedError = describeError(e, "Couldn't read unclaimed credentials");
    }
  }

  async function exportAccounts() {
    try {
      const path = await save({ defaultPath: `haryx-export-${today()}.json`, filters: JSON_FILTER });
      if (!path) return;
      // Every account ticked: export all, so accounts added later are not left out (as macOS).
      const some = chosen.length < store.accounts.length ? ["--account", chosen.join(",")] : [];
      const args = ["export", path, ...some, ...(exportFull ? ["--full"] : [])];
      await store.run("Exporting", args, { success: `Exported to ${fileName(path)}` });
    } catch (e) {
      store.fail(e, "Couldn't export");
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
      store.fail(e, "Couldn't import");
    }
  }

  async function importUsage() {
    try {
      const path = await pickJson();
      if (!path) return;
      const hold = Math.max(0, Math.min(HOLD_MAX_S, Math.round(holdSeconds || 0)));
      await store.run("Importing usage", ["import-usage", path, ...(hold > 0 ? ["--hold", String(hold)] : [])]);
    } catch (e) {
      store.fail(e, "Couldn't import usage");
    }
  }

  const importCswap = () =>
    store.run("Importing from cswap", ["import", "--from-cswap"], { success: "Accounts imported from cswap" });

  async function purgeEntry(u: Unclaimed) {
    const ok = await ask(`Delete the stashed credential ${u.id} (account ${u.slot})? Recovery is signing in again.`, {
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
  const reveal = (what: "store" | "log") =>
    api.reveal(what).catch((e) => store.fail(e, what === "log" ? "Couldn't show the log" : "Couldn't open the data folder"));

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
      store.fail(e, "Couldn't stop auto-switch before erasing");
      return;
    }
    await store.run("Purging all data", ["purge"], { stdin: "y\n", success: "All account data removed" });
    await loadUnclaimed();
  }

  const holdLabel = (s: number) =>
    s === 0 ? "Read usage again right after importing" : `Use the imported readings for ${s / 60} min before reading usage again`;

  function toggleExport(n: number, on: boolean) {
    exportSlots = on ? [...chosen.filter((x) => x !== n), n].sort((a, b) => a - b) : chosen.filter((x) => x !== n);
  }

  onMount(loadUnclaimed);
</script>

<section aria-labelledby="t-export">
  <h2 id="t-export">Export</h2>
  <ul class="rows">
    {#each store.accounts as a (a.number)}
      <li>
        <label for="exp-{a.number}" class="grow truncate" title={a.email}>{a.number}: {middleTruncate(displayName(a), NAME_MAX)}</label>
        <input
          id="exp-{a.number}"
          type="checkbox"
          role="switch"
          class="switch"
          checked={chosen.includes(a.number)}
          onchange={(e) => toggleExport(a.number, e.currentTarget.checked)}
        />
      </li>
    {/each}
  </ul>
  <div class="line">
    <button onclick={() => (exportSlots = store.accounts.map((a) => a.number))}>Select all</button>
    <button onclick={() => (exportSlots = [])}>Select none</button>
    <span class="spacer"></span>
    <span class="caption" role="status">{chosen.length} of {store.accounts.length} selected</span>
  </div>
  <label class="line between">
    <span>Full export (whole .claude.json and credentials, same-machine backup)</span>
    <input type="checkbox" role="switch" class="switch" bind:checked={exportFull} />
  </label>
  <div><button disabled={!!store.busy || chosen.length === 0} onclick={exportAccounts}>Export…</button></div>
  <p class="caption">Exports are plain JSON that includes login tokens. Keep the file somewhere safe or encrypt it.</p>
</section>

<section aria-labelledby="t-import">
  <h2 id="t-import">Import</h2>
  <label class="line between" title="Imports over an account with the same email instead of skipping it (--force).">
    <span>Replace accounts that already exist</span>
    <input type="checkbox" role="switch" class="switch" bind:checked={importForce} />
  </label>
  <div class="line">
    <button disabled={!!store.busy} onclick={importAccounts}>Import accounts…</button>
    <button disabled={!!store.busy} onclick={importCswap} title="Copy the accounts stored by cswap into haryx">Import from cswap</button>
  </div>
  <div
    class="line between"
    title="Imported readings stand in for this long, so this PC doesn't also poll the usage endpoint (--hold)."
  >
    <span id="hold-label">{holdLabel(holdSeconds)}</span>
    <input
      type="number"
      class="num"
      aria-labelledby="hold-label"
      min="0"
      max={HOLD_MAX_S}
      step={HOLD_STEP_S}
      bind:value={holdSeconds}
      aria-valuetext={holdLabel(holdSeconds)}
    />
  </div>
  <div>
    <button disabled={!!store.busy} onclick={importUsage} title="Feed haryx ai list --json output from another machine so both don't poll the usage endpoint"
      >Import usage from another machine…</button
    >
  </div>
</section>

<section aria-labelledby="t-unclaimed">
  <div class="line">
    <h2 id="t-unclaimed">Unclaimed credentials</h2>
    <button class="plain icon-btn" aria-label="Refresh unclaimed credentials" title="Refresh" onclick={loadUnclaimed}>
      <Icon name="refresh" size={14} />
    </button>
  </div>
  {#if unclaimedError}
    <div class="banner error" role="alert" title={unclaimedError.detail}>{unclaimedError.text}</div>
  {:else if unclaimed == null}
    <p class="secondary" role="status">Loading…</p>
  {:else if unclaimed.length === 0}
    <p class="secondary">None</p>
  {:else}
    <ul class="rows">
      {#each unclaimed as u (u.id)}
        <li>
          <div class="grow">
            <div class="mono truncate">{u.id}</div>
            <div class="caption">Account {u.slot} · {u.reason}</div>
          </div>
          <button class="mini danger" disabled={!!store.busy} aria-label="Purge {u.id}" onclick={() => purgeEntry(u)}>Purge</button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<section aria-labelledby="t-haryx">
  <h2 id="t-haryx">HaryX CLI</h2>
  <p class="caption truncate" title={store.cliPath ?? ""}>Executable: {store.cliPath ?? "not found"}</p>
  {#if store.storeRoot}<p class="caption truncate" title={store.storeRoot}>Data folder: {store.storeRoot}</p>{/if}
  <div class="line">
    <button disabled={!!store.busy || !store.cliPath} onclick={upgrade}>Upgrade haryx</button>
    <button onclick={() => reveal("store")}>Open data folder</button>
    <button onclick={() => reveal("log")}>Show log</button>
  </div>
</section>

<section aria-labelledby="t-purge" class="erase">
  <h2 id="t-purge">Erase</h2>
  <p class="caption">Removes every stored account, backup and setting. Your current Claude Code login stays as it is.</p>
  <div class="line">
    <label>Type {PURGE_WORD} to confirm <input bind:value={purgeText} placeholder={PURGE_WORD} autocomplete="off" /></label>
    <button class="danger" disabled={purgeText !== PURGE_WORD || !!store.busy} onclick={purgeAll}>Purge all account data…</button>
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
    margin-bottom: 24px;
  }
  /* Erasing sits last and apart from the routine tools above it. */
  .erase {
    margin-top: 16px;
    padding-top: 16px;
    border-top: 1px solid var(--separator);
  }
  .between {
    justify-content: space-between;
    flex-wrap: nowrap;
  }
  .spacer {
    flex: 1;
  }
  .icon-btn {
    display: inline-flex;
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
  .mono {
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .rows li {
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
  .danger:not(:disabled),
  .mini.danger:not(:disabled) {
    color: var(--red);
    font-weight: 600;
  }
</style>
