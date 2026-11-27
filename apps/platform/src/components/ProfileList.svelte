<script lang="ts">
  // Profiles over unlocked data: names from the encrypted `profiles` index, files from
  // `GET /api/v1/profiles`. A download is opened here and saved as the file `haryx sync profile
  // pull` would import (`<name>.haryx-profile`).
  import { onMount } from "svelte";
  import State from "./State.svelte";
  import { api } from "../lib/auth.svelte";
  import { ApiError } from "../lib/api";
  import { syncLock } from "../lib/synclock";
  import { fileName, joinProfiles, openDownloaded, ProfileError, size, type ProfileItem, type ServerProfile } from "../lib/profiles";
  import { exactTime } from "../lib/machines";
  import type { Synced } from "../lib/synced";

  let { synced }: { synced: Synced } = $props();

  let items = $state.raw<ProfileItem[] | null>(null);
  let unnamed = $state(0);
  let failed = $state(false);
  let busy = $state<string | null>(null);
  let errors = $state<Record<string, string>>({});

  async function load() {
    failed = false;
    items = null;
    const key = syncLock.key;
    if (!key) return;
    try {
      const server = await api<ServerProfile[]>("/profiles");
      const joined = await joinProfiles(key, synced.profiles, server);
      items = joined.items;
      unnamed = joined.unnamed;
    } catch (e) {
      if (!(e instanceof ApiError && e.kind === "unauthorized")) failed = true;
    }
  }
  onMount(load);

  async function download(p: ProfileItem) {
    const key = syncLock.key;
    if (!key) return;
    busy = p.objectKey;
    errors = { ...errors, [p.objectKey]: "" };
    try {
      const cipher = await api<Uint8Array>(`/profiles/${p.objectKey}`, {}, "bytes");
      const plain = await openDownloaded(key, p.name, cipher);
      const url = URL.createObjectURL(new Blob([plain as BlobPart], { type: "application/octet-stream" }));
      const a = Object.assign(document.createElement("a"), { href: url, download: fileName(p.name) });
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      const message =
        e instanceof ProfileError ? e.message
        : e instanceof ApiError && e.kind === "not_found" ? "This file is no longer on the server."
        : "Could not download this profile. Try again.";
      errors = { ...errors, [p.objectKey]: message };
    } finally {
      busy = null;
    }
  }
</script>

{#if items === null && !failed}
  <State kind="loading" />
{:else if failed}
  <State kind="error" title="Could not load your profiles" onAction={load} />
{:else if items!.length === 0}
  <State kind="empty" title="No synced profiles" detail="Upload one from a machine with: haryx sync profile push NAME. Up to 3 are kept." />
{:else}
  <ul>
    {#each items! as p (p.objectKey)}
      <li>
        <div class="info">
          <strong>{p.name}</strong>
          <span class="meta">{size(p.bytes)} · uploaded <time datetime={new Date(p.pushedAt * 1000).toISOString()}>{exactTime(new Date(p.pushedAt * 1000).toISOString())}</time></span>
          {#if p.storedBytes === null}<span class="err">The file is missing on the server.</span>{/if}
          {#if errors[p.objectKey]}<span class="err" role="alert">{errors[p.objectKey]}</span>{/if}
        </div>
        <button onclick={() => download(p)} disabled={busy !== null || p.storedBytes === null}>{busy === p.objectKey ? "Downloading…" : "Download"}</button>
      </li>
    {/each}
  </ul>
  <p class="note">Import a download on a machine with: haryx ai profile import FILE. Secrets were redacted and history left out when it was uploaded.</p>
{/if}
{#if items && unnamed > 0}
  <p class="note">{unnamed} stored {unnamed === 1 ? "file is" : "files are"} not named in your encrypted profile list, so {unnamed === 1 ? "it" : "they"} cannot be opened here.</p>
{/if}

<style>
  ul { list-style: none; margin: 0; padding: 0; display: grid; gap: .6rem; max-width: 52rem; }
  li { background: var(--surface); border: 1px solid var(--separator); border-radius: var(--radius-card); padding: .8rem 1rem; display: flex; gap: .75rem; align-items: center; justify-content: space-between; flex-wrap: wrap; }
  .info { display: grid; gap: .15rem; min-width: 0; flex: 1 1 14rem; }
  strong { overflow-wrap: anywhere; }
  .meta, .note { color: var(--label-2); font-size: .85rem; }
  .note { max-width: 52rem; margin-top: .75rem; overflow-wrap: anywhere; }
  .err { color: var(--red); font-size: .85rem; }
</style>
