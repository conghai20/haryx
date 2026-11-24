<script lang="ts">
  // `haryx ai profile`: saved profiles, the inventory of the selected one (default = the live
  // .claude), and inline forms for save, export, import, apply (dry-run plan first) and diff.
  import { onMount } from "svelte";
  import { ask, open, save } from "@tauri-apps/plugin-dialog";
  import { api } from "../lib/api";
  import { store } from "../lib/store.svelte";

  interface Summary { name: string; created_at: string; total_bytes: number; secrets: string; plugins: string; has_credentials: boolean; source_os: string; last_applied: boolean }
  interface Item { category: string; id: string; kind: string; size: number; files: number; status: string; detail: string[] }
  interface PlanEntry { category: string; id: string; action: string; note: string | null; needs_secret: string[] }
  interface Plan { entries: PlanEntry[]; runs_code: string[]; missing_requires: string[]; warnings: string[] }
  interface DiffItem { category: string; id: string; state: string }
  type Mode = "view" | "save" | "export" | "import" | "apply" | "diff";

  const DEFAULT = "default";
  const CATEGORIES: [string, string, boolean][] = [
    ["settings", "Settings", true], ["hooks", "Hooks", true], ["skills", "Skills", true], ["agents", "Agents", true],
    ["commands", "Commands", true], ["rules", "Rules", true], ["output-styles", "Output styles", true],
    ["plugins", "Plugins", true], ["mcps", "MCP servers", true], ["memory", "Memory (CLAUDE.md)", true],
    ["other", "Other files", true], ["history", "History", true], ["credentials", "Credentials", false],
    ["signed", "Signed files", false], ["claude-json", "Whole .claude.json", false], ["cache", "Caches", false],
  ];
  const CHANGES = ["add", "update", "merge", "remove"];
  const FILTER = [{ name: "HaryX profile", extensions: ["haryx-profile"] }];

  let profiles = $state<Summary[]>([]);
  let selected = $state(DEFAULT);
  let items = $state<Item[] | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(false);
  let mode = $state<Mode>("view");
  // Save / export options (CLI defaults).
  let name = $state("");
  let force = $state(false);
  let history = $state(true);
  let bundle = $state(false);
  let credentials = $state(false);
  let claudeJson = $state(false);
  let signed = $state(false);
  let cache = $state(false);
  let dirs = $state<string[]>([]);
  let importPath = $state("");
  // Apply / diff.
  let prune = $state(false);
  let plan = $state<Plan | null>(null);
  let diff = $state<DiffItem[] | null>(null);
  let sideError = $state<string | null>(null);

  let summary = $derived(profiles.find((p) => p.name === selected) ?? null);
  let groups = $derived(CATEGORIES.map(([c, title, on]) => ({ c, title: on ? title : `${title} (opt-in)`, list: (items ?? []).filter((i) => i.category === c) })).filter((g) => g.list.length > 0));
  let changes = $derived(plan?.entries.filter((e) => CHANGES.includes(e.action)) ?? []);
  let encrypted = $derived(summary?.secrets === "encrypt");
  /** Opens an encrypted profile; passed to the CLI in its environment only, never kept. */
  let passphrase = $state("");
  let nameOk = $derived(/^[A-Za-z0-9_-][A-Za-z0-9._-]{0,63}$/.test(name) && name !== DEFAULT);

  const titleOf = (c: string) => CATEGORIES.find(([n]) => n === c)?.[1] ?? c;
  const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;
  function bytes(n: number): string {
    const units = ["bytes", "KB", "MB", "GB"];
    let v = n, u = 0;
    while (v >= 1024 && u < units.length - 1) { v /= 1024; u++; }
    return u === 0 ? `${n} bytes` : `${v.toFixed(1)} ${units[u]}`;
  }
  const partArgs = (withCredentials: boolean) => [
    ...(history ? [] : ["--no-history"]), ...(withCredentials && credentials ? ["--credentials"] : []),
    ...(signed ? ["--signed"] : []), ...(claudeJson ? ["--claude-json"] : []), ...(cache ? ["--cache"] : []),
    ...dirs.flatMap((d) => ["--include-dir", d]), "--plugins", bundle ? "bundle" : "ref",
  ];

  async function reload() {
    loading = true;
    try {
      profiles = (await api.json<{ profiles: Summary[] }>(["ai", "profile", "list", "--json"])).profiles;
      if (selected !== DEFAULT && !profiles.some((p) => p.name === selected)) selected = DEFAULT;
      items = (await api.json<{ items: Item[] }>(["ai", "profile", "show", selected, "--json"])).items;
      loadError = null;
    } catch (e) {
      loadError = String(e);
    } finally {
      loading = false;
    }
  }

  function select(n: string) {
    selected = n;
    mode = "view";
    void reload();
  }

  function start(m: Mode) {
    mode = m;
    sideError = null;
    force = false;
    if (m === "save") name = "";
    if (m === "apply") void loadPlan();
    if (m === "diff") void loadDiff();
  }

  async function loadPlan() {
    plan = null;
    try {
      plan = await api.json<Plan>(["ai", "profile", "apply", selected, "--dry-run", "--json", ...(prune ? ["--prune"] : []), ...(history ? [] : ["--no-history"])]);
      sideError = null;
    } catch (e) { sideError = String(e); }
  }

  async function loadDiff() {
    diff = null;
    try {
      diff = (await api.json<{ items: DiffItem[] }>(["ai", "profile", "diff", selected, "--json"])).items;
      sideError = null;
    } catch (e) { sideError = String(e); }
  }

  async function addDir() {
    try {
      const d = await open({ directory: true, multiple: false });
      if (typeof d === "string" && !dirs.includes(d)) dirs = [...dirs, d];
    } catch (e) { store.flash(String(e), true); }
  }

  async function saveProfile() {
    const n = name;
    if (await store.run("Saving profile", ["ai", "profile", "save", n, ...partArgs(true), ...(force ? ["--force"] : [])], { success: `Saved profile '${n}'` })) select(n);
  }

  async function exportProfile() {
    try {
      const path = await save({ defaultPath: `${selected}-${new Date().toISOString().slice(0, 10)}.haryx-profile`, filters: FILTER });
      if (!path) return;
      if (await store.run("Exporting profile", ["ai", "profile", "export", selected, path, ...partArgs(false), "--secrets", "redact"], { success: `Exported to ${fileName(path)}` })) mode = "view";
    } catch (e) { store.flash(String(e), true); }
  }

  async function pickImport() {
    try {
      const path = await open({ multiple: false, directory: false, filters: FILTER });
      if (typeof path !== "string") return;
      importPath = path;
      const base = fileName(path).replace(/\.haryx-profile$/, "").replace(/[^A-Za-z0-9._-]/g, "-");
      name = base && base !== DEFAULT && !base.startsWith(".") ? base.slice(0, 64) : "imported";
      start("import");
    } catch (e) { store.flash(String(e), true); }
  }

  async function importProfile() {
    const n = name;
    if (!(await store.run("Importing profile", ["ai", "profile", "import", n, importPath, ...(force ? ["--force"] : [])], { success: `Imported '${n}'. Review the plan before applying.` }))) return;
    selected = n;
    await reload();
    start("apply");
  }

  async function applyProfile() {
    const ok = await ask(`Apply '${selected}' to your live .claude? ${changes.length} change(s). A backup is made first; Undo last apply restores it.`, { title: "Apply profile", kind: "warning", okLabel: "Apply", cancelLabel: "Cancel" });
    if (!ok) return;
    if (await store.run("Applying profile", ["ai", "profile", "apply", selected, "--yes", ...(prune ? ["--prune"] : []), ...(history ? [] : ["--no-history"])], { success: `Applied '${selected}'`, passphrase: encrypted ? passphrase : undefined })) {
      passphrase = "";
      mode = "view";
      await reload();
    }
  }

  async function deleteProfile() {
    const ok = await ask(`Delete profile '${selected}'? The saved copy is removed. Your live .claude is not touched.`, { title: "Delete profile", kind: "warning", okLabel: "Delete", cancelLabel: "Cancel" });
    if (ok && (await store.run("Deleting profile", ["ai", "profile", "delete", selected, "--yes"], { success: `Deleted profile '${selected}'` }))) select(DEFAULT);
  }

  async function rollback() {
    const ok = await ask("Undo the last apply? Files and settings it changed are restored. Installed plugins and imported accounts stay.", { title: "Undo last apply", kind: "warning", okLabel: "Restore backup", cancelLabel: "Cancel" });
    if (ok && (await store.run("Restoring backup", ["ai", "profile", "rollback"]))) await reload();
  }

  onMount(reload);
</script>

<div class="bar">
  <label class="sr" for="profile-pick">Profile</label>
  <select id="profile-pick" value={selected} onchange={(e) => select(e.currentTarget.value)}>
    <option value={DEFAULT}>default (live .claude)</option>
    {#each profiles as p (p.name)}<option value={p.name}>{p.name}{p.last_applied ? " (last applied)" : ""}</option>{/each}
  </select>
  {#if summary}<span class="caption truncate">{bytes(summary.total_bytes)} · from {summary.source_os} · secrets {summary.secrets}{summary.has_credentials ? " · credentials" : ""}</span>{/if}
  <span class="spacer"></span>
  <button class="plain" aria-label="Refresh profiles" title="Refresh" disabled={loading} onclick={reload}>⟳</button>
  <button onclick={() => start("save")}>Save snapshot…</button>
  <button onclick={() => start("export")}>Export…</button>
  <button onclick={pickImport}>Import…</button>
  {#if selected !== DEFAULT}
    <button onclick={() => start("diff")}>Diff</button>
    <button class="primary" onclick={() => start("apply")}>Apply…</button>
    <button title="Delete this saved profile" onclick={deleteProfile}>Delete…</button>
  {/if}
  <button class="plain" aria-label="Undo the last apply" title="Undo the last apply" onclick={rollback}>↶</button>
</div>

{#if mode !== "view"}
  <section class="panel" aria-label="Profile action">
    {#if mode === "save" || mode === "import"}
      <label class="line">{mode === "save" ? "Name" : `Save ${fileName(importPath)} as`} <input bind:value={name} placeholder="work" aria-invalid={!nameOk} /></label>
      <label class="line"><input type="checkbox" bind:checked={force} /> Replace a profile with this name</label>
      {#if mode === "import"}<p class="caption">Importing only stores the profile. You review what it changes before anything is applied.</p>{/if}
    {/if}
    {#if mode === "save" || mode === "export"}
      <fieldset>
        <legend class="caption">Include</legend>
        <label class="line"><input type="checkbox" bind:checked={history} /> History (conversations, prompt history, checkpoints)</label>
        <label class="line"><input type="checkbox" bind:checked={bundle} /> Plugin files (otherwise only names, reinstalled on apply)</label>
        {#if mode === "save"}<label class="line"><input type="checkbox" bind:checked={credentials} /> Accounts and logins</label>{/if}
        <label class="line"><input type="checkbox" bind:checked={claudeJson} /> Whole .claude.json (identity, folder trust)</label>
        <label class="line"><input type="checkbox" bind:checked={signed} /> Anthropic-signed files (policy limits, remote settings)</label>
        <label class="line"><input type="checkbox" bind:checked={cache} /> Caches</label>
        {#each dirs as d (d)}
          <div class="line"><code class="truncate">{d}</code><button class="plain" aria-label="Remove folder {d}" onclick={() => (dirs = dirs.filter((x) => x !== d))}>✕</button></div>
        {/each}
        <div><button onclick={addDir}>Add folder…</button></div>
      </fieldset>
      {#if mode === "export"}<p class="caption">Secrets in MCP servers, settings and history are redacted; set them again on the other machine.</p>{/if}
    {/if}
    {#if mode === "apply"}
      <label class="line"><input type="checkbox" bind:checked={history} onchange={loadPlan} /> Include history</label>
      <label class="line"><input type="checkbox" bind:checked={prune} onchange={loadPlan} /> Remove local items the profile does not have (history is always kept)</label>
      {#if encrypted}
        <label class="line">Passphrase <input type="password" bind:value={passphrase} autocomplete="off" aria-label="Profile passphrase" /></label>
        <p class="caption secondary">This profile's secrets are encrypted. The passphrase goes to haryx for this apply only.</p>
      {/if}
      {#if sideError}<div class="banner error" role="alert">{sideError}</div>
      {:else if !plan}<p class="secondary" role="status">Planning…</p>
      {:else}
        {#if plan.runs_code.length > 0}<div class="banner error"><strong>Installs code that runs on this PC</strong>{#each plan.runs_code as c (c)}<div><code>{c}</code></div>{/each}</div>{/if}
        {#if plan.missing_requires.length > 0}<p class="warn">Not installed here: {plan.missing_requires.join(", ")}</p>{/if}
        {#each plan.warnings as w (w)}<p class="warn">{w}</p>{/each}
        <p class="caption">{changes.length} change(s), {plan.entries.length - changes.length} unchanged. A backup is made first.</p>
        <table>
          <tbody>
            {#each changes as e (e.category + e.id)}
              <tr><td class="act" class:remove={e.action === "remove"}>{e.action}</td><td class="tertiary">{titleOf(e.category)}</td><td><code class="truncate">{e.id}</code></td>
                <td class="caption">{e.needs_secret.length ? `needs ${e.needs_secret.join(", ")}` : (e.note ?? "")}</td></tr>
            {:else}<tr><td class="secondary">Nothing to change.</td></tr>{/each}
          </tbody>
        </table>
      {/if}
    {/if}
    {#if mode === "diff"}
      {#if sideError}<div class="banner error" role="alert">{sideError}</div>
      {:else if !diff}<p class="secondary" role="status">Comparing…</p>
      {:else}
        <p class="caption">'{selected}' compared with the live .claude: {diff.filter((d) => d.state === "same").length} the same.</p>
        <table><tbody>
          {#each diff.filter((d) => d.state !== "same") as d (d.category + d.id)}
            <tr><td class="act">{d.state === "only-left" ? `only in ${selected}` : d.state === "only-right" ? "only live" : d.state}</td><td class="tertiary">{titleOf(d.category)}</td><td><code class="truncate">{d.id}</code></td></tr>
          {:else}<tr><td class="secondary">No differences.</td></tr>{/each}
        </tbody></table>
      {/if}
    {/if}
    <div class="line end">
      <button onclick={() => (mode = "view")}>{mode === "diff" ? "Close" : "Cancel"}</button>
      {#if mode === "save"}<button class="primary" disabled={!nameOk || !!store.busy} onclick={saveProfile}>Save</button>{/if}
      {#if mode === "import"}<button class="primary" disabled={!nameOk || !!store.busy} onclick={importProfile}>Import</button>{/if}
      {#if mode === "export"}<button class="primary" disabled={!!store.busy} onclick={exportProfile}>Export…</button>{/if}
      {#if mode === "apply"}<button class="primary" disabled={changes.length === 0 || (encrypted && passphrase.length === 0) || !!store.busy} onclick={applyProfile}>Apply</button>{/if}
    </div>
  </section>
{/if}

{#if loadError}
  <div class="banner error" role="alert">Could not read the profile: {loadError}</div>
{:else if !items}
  <p class="secondary" role="status">Loading…</p>
{:else if groups.length === 0}
  <p class="secondary">This profile is empty.</p>
{:else}
  {#each groups as g (g.c)}
    <h2 class="caption">{g.title}</h2>
    {#each g.list as it (it.id)}
      <div class="item">
        <div class="line"><code class="truncate grow" title={it.id}>{it.id}</code>
          {#if it.status !== "ok"}<span class="badge" class:broken={it.status === "broken"} class:amber={it.status !== "broken"}>{it.status}</span>{/if}
          {#if it.files > 0}<span class="caption">{bytes(it.size)} · {it.files} file{it.files === 1 ? "" : "s"}</span>{/if}
        </div>
        {#each it.detail as line, i (i)}<div class="caption detail"><code>{line}</code></div>{/each}
      </div>
    {/each}
  {/each}
{/if}

<style>
  .bar, .line { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .bar { flex-wrap: wrap; margin-bottom: 10px; }
  .spacer, .grow { flex: 1; min-width: 0; }
  .end { justify-content: flex-end; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
  .panel { display: flex; flex-direction: column; gap: 8px; background: var(--fill); border-radius: var(--radius-card); padding: 14px; margin-bottom: 12px; }
  .panel p { margin: 0; }
  fieldset { border: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  code { font-family: ui-monospace, "Cascadia Mono", Consolas, monospace; }
  table { width: 100%; border-collapse: collapse; table-layout: fixed; }
  td { padding: 4px 6px 4px 0; border-top: 1px solid var(--separator); }
  .act { width: 110px; font-weight: 600; color: var(--cyan-text); }
  .act.remove { color: var(--red); }
  .warn { color: var(--amber); }
  h2 { margin: 14px 0 4px; font-weight: 600; }
  .item { padding: 5px 0; border-top: 1px solid var(--separator); }
  .detail { padding-left: 16px; }
  .badge.broken { color: var(--red); background: color-mix(in srgb, var(--red) 18%, transparent); }
  .badge.amber { color: var(--amber); background: color-mix(in srgb, var(--amber) 18%, transparent); }
</style>
