<script lang="ts">
  // The dashboard sidebar's footer: the HaryX account. Signed out, a menu of ways to sign in;
  // signing in, progress and Cancel; signed in, who it is and a menu to manage the account.
  import { onMount } from "svelte";
  import { SIGN_IN_PROVIDERS, providerName, type SignInProvider } from "../lib/account";
  import { haryxAccount as acct } from "../lib/haryx.svelte";
  import { store } from "../lib/store.svelte";

  let { onopen }: { onopen: () => void } = $props();

  let open = $state(false);
  let root = $state<HTMLElement | null>(null);
  let user = $derived(acct.signedIn ? (acct.whoami?.user ?? null) : null);
  let status = $derived(
    acct.syncing
      ? "Syncing…"
      : acct.problem
        ? "Sync needs attention"
        : !acct.unlocked
          ? "Sync locked"
          : store.prefs?.syncEnabled === false
            ? "Sync off"
            : "Sync on",
  );

  onMount(() => {
    void acct.init();
    const close = (e: MouseEvent) => {
      if (open && root && !root.contains(e.target as Node)) open = false;
    };
    const escape = (e: KeyboardEvent) => {
      if (e.key === "Escape") open = false;
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", escape);
    };
  });

  function choose(action: () => void) {
    open = false;
    action();
  }

  function signIn(provider: SignInProvider) {
    choose(() => void acct.startSignIn(provider));
  }
</script>

{#if acct.whoami}
  <div class="footer" bind:this={root}>
    {#if user}
      {#if open}
        <div class="menu" role="menu" aria-label="HaryX account">
          <p class="menu-head caption tertiary">Signed in with {providerName(user.provider)}</p>
          <button role="menuitem" onclick={() => choose(onopen)}>Account &amp; sync…</button>
          <button role="menuitem" onclick={() => choose(() => void acct.signOut())}>Sign out</button>
        </div>
      {/if}
      <button class="who" aria-haspopup="menu" aria-expanded={open} onclick={() => (open = !open)}>
        <span class="avatar" aria-hidden="true">{user.email.charAt(0).toUpperCase()}</span>
        <span class="text">
          <span class="truncate">{user.email}</span>
          <span class="caption secondary truncate">{status}</span>
        </span>
        <span class="chevron" aria-hidden="true">⌃</span>
      </button>
    {:else if acct.signIn.phase === "waiting"}
      <div class="waiting" role="status">
        <span class="spinner" aria-hidden="true"></span>
        <span class="caption secondary truncate">Signing in with {providerName(acct.signInProvider)}…</span>
        {#if acct.signIn.hasLink}
          <button class="icon-button" title="Open the sign-in page again" onclick={() => acct.openSignInPage()}>↗</button>
        {/if}
        <button class="icon-button" title="Cancel sign-in" onclick={() => acct.cancelSignIn()}>✕</button>
      </div>
    {:else}
      {#if acct.signIn.phase === "error"}
        <p class="caption error-text" role="alert">{acct.signIn.message}</p>
      {/if}
      {#if open}
        <div class="menu" role="menu" aria-label="Sign in to HaryX">
          {#each SIGN_IN_PROVIDERS as p (p.id)}
            <button role="menuitem" disabled={!p.available} onclick={() => signIn(p.id)}>
              {p.available ? `Continue with ${p.label}` : `${p.label} (coming soon)`}
            </button>
          {/each}
        </div>
      {/if}
      <button
        class="sign-in"
        aria-haspopup="menu"
        aria-expanded={open}
        title="Sign in to sync settings between your machines"
        onclick={() => (open = !open)}
      >
        <span aria-hidden="true">◉</span>Sign in to HaryX<span class="chevron" aria-hidden="true">⌃</span>
      </button>
    {/if}
  </div>
{/if}

<style>
  .footer {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 8px;
    border-top: 1px solid var(--separator);
  }
  .sign-in,
  .who {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    text-align: left;
  }
  .sign-in {
    background: var(--fill-strong);
    padding: 6px 8px;
  }
  .who {
    background: transparent;
    padding: 6px 8px;
  }
  .who:hover,
  .who[aria-expanded="true"] {
    background: var(--fill);
  }
  .avatar {
    flex: none;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--cyan) 22%, transparent);
    color: var(--label);
    font-weight: 600;
    font-size: 12px;
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .chevron {
    margin-left: auto;
    color: var(--label-2);
    font-size: 11px;
  }
  .menu {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(100% + 4px);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-banner);
    padding: 4px;
    z-index: 2;
  }
  .menu button {
    text-align: left;
    background: transparent;
  }
  .menu button:hover:not(:disabled) {
    background: var(--fill);
  }
  .menu-head {
    margin: 2px 8px 4px;
  }
  .waiting {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
  }
  .waiting .truncate {
    flex: 1;
  }
  .icon-button {
    background: transparent;
    padding: 2px 6px;
  }
  .error-text {
    color: var(--red);
    margin: 0 8px;
  }
  .spinner {
    flex: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid var(--track);
    border-top-color: var(--cyan);
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
