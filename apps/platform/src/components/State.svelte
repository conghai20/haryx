<script lang="ts">
  // One component for every non-success state a section can be in: plain text on the page, never a
  // boxed card. Sections pass the kind; the wording lives here so every section says the same thing.
  import type { Snippet } from "svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Skeleton } from "$lib/components/ui/skeleton/index.js";

  type Kind = "loading" | "empty" | "error" | "locked" | "consentMissing";
  let {
    kind,
    title,
    detail,
    onAction,
    actionLabel,
    children,
  }: { kind: Kind; title?: string; detail?: string; onAction?: () => void; actionLabel?: string; children?: Snippet } = $props();

  const copy: Record<Exclude<Kind, "loading">, { title: string; detail: string; action: string }> = {
    empty: { title: "Nothing here yet", detail: "", action: "" },
    error: { title: "Could not load this", detail: "HaryX did not answer. Your data is safe; try again in a moment.", action: "Try again" },
    locked: { title: "Locked", detail: "Enter your sync passphrase to read this. It is never stored.", action: "Unlock" },
    consentMissing: { title: "Consent needed", detail: "Allow this in the Privacy center to use this section.", action: "Open Privacy center" },
  };
  const c = $derived(kind === "loading" ? null : copy[kind]);
</script>

{#if kind === "loading"}
  <div class="border-rule grid border-t" role="status" aria-label="Loading">
    {#each [0, 1, 2] as i (i)}
      <div class="border-rule grid gap-3 border-b py-6"><Skeleton class="h-5 w-1/3" /><Skeleton class="h-3 w-1/2" /></div>
    {/each}
  </div>
{:else if c}
  <div class="border-rule max-w-xl border-t py-10" role={kind === "error" ? "alert" : "status"}>
    <h2 class="text-xl font-semibold">{title ?? c.title}</h2>
    {#if detail ?? c.detail}<p class="text-ink-2 mt-2">{detail ?? c.detail}</p>{/if}
    {#if children}<div class="mt-5">{@render children()}</div>{/if}
    {#if onAction && (actionLabel ?? c.action)}
      <div class="mt-6"><Button variant="outline" onclick={onAction}>{actionLabel ?? c.action}</Button></div>
    {/if}
  </div>
{/if}
