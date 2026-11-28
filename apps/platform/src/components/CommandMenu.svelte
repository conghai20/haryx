<script lang="ts">
  // Cmd/Ctrl+K: jump to a section or an account, or lock, from anywhere.
  import * as Command from "$lib/components/ui/command/index.js";
  import type { Overview } from "../lib/overview";
  import { syncLock } from "../lib/synclock";
  import type { Section } from "../lib/sections";

  let { open = $bindable(false), sections, view, onOpenAccount }: { open?: boolean; sections: readonly Section[]; view: Overview | null; onOpenAccount: (id: string) => void } = $props();

  function go(run: () => void) {
    open = false;
    run();
  }
</script>

<Command.Dialog bind:open title="Jump to" description="Find a section or an account.">
  <Command.Input placeholder="Jump to a section or an account" />
  <Command.List>
    <Command.Empty>Nothing matches.</Command.Empty>
    <Command.Group heading="Sections">
      {#each sections as s (s.id)}
        <Command.Item value={`section ${s.label}`} onSelect={() => go(() => (location.hash = `#${s.id}`))}>{s.label}</Command.Item>
      {/each}
    </Command.Group>
    {#if view?.accounts.length}
      <Command.Group heading="Accounts">
        {#each view.accounts as a (a.id)}
          <Command.Item value={`account ${a.title} ${a.account.email}`} onSelect={() => go(() => onOpenAccount(a.id))}>
            <span class="truncate">{a.title}</span>
            {#if a.account.alias}<span class="text-ink-3 ml-auto truncate text-xs">{a.account.email}</span>{/if}
          </Command.Item>
        {/each}
      </Command.Group>
    {/if}
    {#if syncLock.status === "unlocked"}
      <Command.Group heading="Actions">
        <Command.Item value="lock synced data" onSelect={() => go(() => syncLock.lock())}>Lock synced data</Command.Item>
      </Command.Group>
    {/if}
  </Command.List>
</Command.Dialog>
