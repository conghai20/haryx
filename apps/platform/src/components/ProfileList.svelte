<script lang="ts">
  // Profiles over unlocked data: names from the encrypted `profiles` index, files from
  // `GET /api/v1/profiles`. A download is opened here and saved as the file `haryx sync profile
  // pull` would import (`<name>.haryx-profile`).
  import { onMount } from "svelte";
  import State from "./State.svelte";
  import CommandField from "./CommandField.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
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
  <State kind="empty" title="No synced profiles" detail="Upload a profile from a Mac or PC. Up to 3 are kept.">
    <CommandField command="haryx sync profile push NAME" />
  </State>
{:else}
  <ul class="border-rule max-w-4xl border-b">
    {#each items! as p (p.objectKey)}
      <li class="border-rule flex flex-wrap items-center justify-between gap-x-6 gap-y-3 border-t py-6">
        <div class="grid min-w-0 flex-[1_1_16rem] gap-1">
          <p class="text-[17px] font-semibold break-words">{p.name}</p>
          <p class="text-ink-3 text-sm">{size(p.bytes)} · uploaded <time datetime={new Date(p.pushedAt * 1000).toISOString()}>{exactTime(new Date(p.pushedAt * 1000).toISOString())}</time></p>
          {#if p.storedBytes === null}<p class="text-red-text text-sm">The file is missing on the server.</p>{/if}
          {#if errors[p.objectKey]}<p class="text-red-text text-sm" role="alert">{errors[p.objectKey]}</p>{/if}
        </div>
        <Button variant="outline" onclick={() => download(p)} disabled={busy !== null || p.storedBytes === null}>{busy === p.objectKey ? "Downloading…" : "Download"}</Button>
      </li>
    {/each}
  </ul>
  <div class="mt-10 grid max-w-2xl gap-3">
    <p class="text-ink-2">Import a download on a Mac or PC. Secrets were redacted and history left out when it was uploaded.</p>
    <CommandField command="haryx ai profile import ACCOUNT FILE.haryx-profile" />
  </div>
{/if}
{#if items && unnamed > 0}
  <p class="text-ink-3 mt-6 max-w-2xl text-sm">{unnamed} stored {unnamed === 1 ? "file is" : "files are"} not named in your encrypted profile list, so {unnamed === 1 ? "it" : "they"} cannot be opened here.</p>
{/if}
