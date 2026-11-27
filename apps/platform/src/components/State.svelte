<script lang="ts">
  // One component for every non-success state a section can be in. Sections pass the kind; the
  // wording lives here so all five sections say the same thing the same way.
  import type { Snippet } from "svelte";

  type Kind = "loading" | "empty" | "error" | "signedOut" | "locked" | "consentMissing";
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
    error: { title: "Could not load this", detail: "Check your connection and try again.", action: "Retry" },
    signedOut: { title: "You are signed out", detail: "Sign in with your WeCoAp account to continue.", action: "Sign in" },
    locked: { title: "Locked", detail: "Enter your sync passphrase to read this. It is never stored.", action: "Unlock" },
    consentMissing: { title: "Consent needed", detail: "Allow this purpose in the Privacy center to use this section.", action: "Open Privacy center" },
  };
  const c = $derived(kind === "loading" ? null : copy[kind]);
</script>

{#if kind === "loading"}
  <div class="skeleton" role="status" aria-label="Loading">
    <span></span><span></span><span></span>
  </div>
{:else if c}
  <div class="state" role={kind === "error" ? "alert" : "status"}>
    <h2>{title ?? c.title}</h2>
    <p>{detail ?? c.detail}</p>
    {@render children?.()}
    {#if onAction && (actionLabel ?? c.action)}
      <button class="primary" onclick={onAction}>{actionLabel ?? c.action}</button>
    {/if}
  </div>
{/if}

<style>
  .state { background: var(--surface); border: 1px solid var(--separator); border-radius: var(--radius-card); padding: 1.25rem; max-width: 36rem; }
  .skeleton { display: grid; gap: .6rem; max-width: 36rem; }
  .skeleton span { display: block; height: 3.2rem; border-radius: var(--radius-card); background: var(--fill-strong); animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: .45; } }
</style>
