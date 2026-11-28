<script lang="ts">
  // The sync passphrase form. The passphrase goes to the unlock worker and is dropped with the
  // form; it is never stored.
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { syncLock } from "../lib/synclock";
  import type { SyncLock } from "../lib/unlock.svelte";

  let { lock = syncLock, compact = false }: { lock?: SyncLock; compact?: boolean } = $props();
  let passphrase = $state("");
  const busy = $derived(lock.status === "unlocking");
  const wrong = $derived(lock.error?.code === "wrong_passphrase");

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!passphrase || busy) return;
    const typed = passphrase;
    passphrase = "";
    await lock.unlock(typed);
  }
</script>

<form class="grid gap-3" onsubmit={submit} aria-busy={busy}>
  <Label for="sync-passphrase" class="text-ink-2 text-sm font-medium">Sync passphrase</Label>
  <div class="flex flex-wrap gap-2">
    <Input
      id="sync-passphrase"
      type="password"
      class="min-w-0 flex-[1_1_14rem] bg-transparent {compact ? '' : 'h-12 text-base'}"
      autocomplete="off"
      autocapitalize="off"
      spellcheck="false"
      bind:value={passphrase}
      disabled={busy}
      aria-invalid={wrong ? "true" : undefined}
      aria-describedby={lock.error ? "unlock-error" : "unlock-note"}
    />
    <Button type="submit" size={compact ? "default" : "lg"} disabled={busy || !passphrase}>{busy ? "Unlocking…" : "Unlock"}</Button>
  </div>
  {#if busy}
    <p class="text-ink-2 flex items-center gap-3 text-sm" role="status">
      <span class="unlock-bar bg-track relative block h-[3px] w-20 overflow-hidden rounded-[1.5px]"><span class="bg-live absolute inset-y-0 w-1/3"></span></span>
      Opening your data in this tab. This takes a second or two.
    </p>
  {:else if lock.error}
    <div id="unlock-error" class="grid gap-1 text-sm" role="alert">
      <p class="text-red-text font-medium">{lock.error.message}</p>
      {#if wrong}
        <p class="text-ink-3">Forgot it? HaryX cannot recover it, but any Mac or PC that is still unlocked can set a new one in HaryX, under HaryX Account.</p>
      {/if}
    </div>
  {/if}
  <p id="unlock-note" class="text-ink-3 text-sm">Decrypted in this tab only, never stored. It locks again after 15 minutes idle or when you close the tab.</p>
</form>

<style>
  @media (prefers-reduced-motion: no-preference) {
    .unlock-bar > span { animation: slide 1.1s cubic-bezier(0.65, 0, 0.35, 1) infinite; }
  }
  @keyframes slide { from { transform: translateX(-100%); } to { transform: translateX(300%); } }
</style>
