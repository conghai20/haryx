<script lang="ts">
  // HaryX › HaryX Account: sign in with GitHub, unlock sync with the passphrase, then sync settings,
  // preferences and account setup between machines. Claude logins never leave this PC.
  import { onMount } from "svelte";
  import { MIN_PASSPHRASE, PASSPHRASE_WARNING, overwriteLine, passphraseProblem, providerName, syncedAgo } from "../lib/account";
  import { haryxAccount as acct } from "../lib/haryx.svelte";
  import { store } from "../lib/store.svelte";

  let passphrase = $state("");
  let repeat = $state("");
  let formError = $state<string | null>(null);
  let working = $state(false);
  let changing = $state(false);

  let creating = $derived(acct.hasPassphrase === false);
  let user = $derived(acct.whoami?.user ?? null);
  let overwritten = $derived((acct.last?.ok ? acct.last.overwritten : acct.status?.overwritten) ?? []);

  onMount(() => {
    void acct.init();
  });

  function resetForm() {
    passphrase = "";
    repeat = "";
    formError = null;
  }

  async function submit(e: SubmitEvent, action: (p: string) => Promise<string | null>, twice: boolean) {
    e.preventDefault();
    formError = twice ? passphraseProblem(passphrase, repeat) : passphrase ? null : "Enter your sync passphrase.";
    if (formError) return;
    working = true;
    try {
      formError = await action(passphrase);
      if (!formError) {
        resetForm();
        changing = false;
      }
    } finally {
      working = false;
    }
  }
</script>

{#if acct.loadError && !acct.whoami}
  <div class="banner error" role="alert">
    {acct.loadError}
    <button class="plain" onclick={() => acct.refresh()}>Try again</button>
  </div>
{:else if !acct.whoami}
  <p class="secondary">Loading…</p>
{:else if !acct.signedIn || !user}
  <section aria-labelledby="signin-title">
    <h2 id="signin-title">Sync between your machines</h2>
    <p class="lede">
      Sign in to sync settings, preferences and account setup between your PCs and Macs. Your Claude logins never
      leave this PC.
    </p>
    {#if acct.signIn.phase === "waiting"}
      <div class="row" role="status">
        <span class="spinner" aria-hidden="true"></span>
        <span>
          {acct.signIn.hasLink && !acct.signIn.browserOpened
            ? "Open the sign-in page to continue."
            : "Waiting for you to finish signing in in your browser…"}
        </span>
      </div>
      <div class="row">
        {#if acct.signIn.hasLink}
          <button class:primary={!acct.signIn.browserOpened} onclick={() => acct.openSignInPage()}>Open sign-in page</button>
        {/if}
        <button onclick={() => acct.cancelSignIn()}>Cancel</button>
      </div>
    {:else}
      {#if acct.signIn.phase === "error"}
        <div class="banner error" role="alert">{acct.signIn.message}</div>
      {/if}
      <div class="row">
        <button class="primary" onclick={() => acct.startSignIn()}>
          {acct.signIn.phase === "error" ? "Try again with GitHub" : "Continue with GitHub"}
        </button>
      </div>
    {/if}
    <p class="caption tertiary">Signed out, HaryX sends nothing.</p>
  </section>
{:else}
  <section aria-labelledby="who-title">
    <h2 id="who-title" class="visually-hidden">HaryX account</h2>
    <div class="who">
      <strong class="truncate">{user.email}</strong>
      <span class="caption secondary">Signed in with {providerName(user.provider)}</span>
    </div>
  </section>

  <div class="hairline"></div>

  {#if !acct.unlocked}
    <section aria-labelledby="unlock-title">
      <h2 id="unlock-title">{creating ? "Set a sync passphrase" : "Unlock sync on this PC"}</h2>
      {#if acct.problem?.unlock}
        <p class="caption amber" role="status">{acct.problem.text}</p>
      {/if}
      {#if acct.hasPassphrase == null}
        {#if acct.loadError}
          <div class="banner error" role="alert">
            {acct.loadError}
            <button class="plain" onclick={() => acct.checkPassphrase()}>Try again</button>
          </div>
        {:else}
          <p class="secondary">Checking…</p>
        {/if}
      {:else}
        <p class="secondary">
          {creating
            ? "Everything HaryX syncs is encrypted with this passphrase before it leaves this PC. You enter it once on each machine."
            : "Enter the sync passphrase you set on your other machine."}
        </p>
        <form onsubmit={(e) => submit(e, (p) => acct.unlock(p), creating)}>
          <div class="field">
            <label for="pass">{creating ? "New passphrase" : "Passphrase"}</label>
            <input id="pass" type="password" autocomplete={creating ? "new-password" : "current-password"} bind:value={passphrase} />
          </div>
          {#if creating}
            <div class="field">
              <label for="pass2">Repeat it</label>
              <input id="pass2" type="password" autocomplete="new-password" bind:value={repeat} />
            </div>
            <p class="caption secondary">At least {MIN_PASSPHRASE} characters. {PASSPHRASE_WARNING}</p>
          {/if}
          {#if formError}<p class="caption error-text" role="alert">{formError}</p>{/if}
          <div class="row">
            <button class="primary" type="submit" disabled={working}>{creating ? "Set passphrase" : "Unlock"}</button>
            {#if working}<span class="caption secondary">This takes a second…</span>{/if}
          </div>
        </form>
      {/if}
    </section>
  {:else if store.prefs}
    {@const prefs = store.prefs}
    <section aria-labelledby="sync-title">
      <h2 id="sync-title">Sync</h2>
      <label class="check">
        <input type="checkbox" checked={prefs.syncEnabled} onchange={(e) => store.savePrefs({ syncEnabled: e.currentTarget.checked })} />
        Sync this PC with your HaryX account
      </label>
      <div class="row" role="status">
        {#if acct.syncing}
          <span class="spinner" aria-hidden="true"></span><span class="secondary">Syncing…</span>
        {:else}
          <span class="secondary">{syncedAgo(acct.status?.lastSyncAt ?? null, store.now)}</span>
          {#if acct.problem}<span class="caption amber">· {acct.problem.text}</span>{/if}
        {/if}
        <button disabled={acct.syncing || !prefs.syncEnabled} onclick={() => acct.syncNow()}>Sync now</button>
      </div>
      <p class="caption tertiary">
        Syncs at launch, every 15 minutes and shortly after a change. This switch stays on this PC.
      </p>
    </section>

    {#if overwritten.length > 0}
      <div class="hairline"></div>
      <section aria-labelledby="over-title">
        <h2 id="over-title">Recent overwrites</h2>
        <p class="caption tertiary">When two machines changed the same thing, the newer change won.</p>
        <ul>
          {#each overwritten.slice(0, 10) as o (`${o.at}-${o.kind}-${o.label}`)}
            <li>
              <span>{overwriteLine(o)}</span>
              <span class="caption tertiary">{new Date(o.at * 1000).toLocaleString()}</span>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <div class="hairline"></div>

    <section aria-labelledby="pass-title">
      <h2 id="pass-title">Passphrase</h2>
      {#if changing}
        <form onsubmit={(e) => submit(e, (p) => acct.changePassphrase(p), true)}>
          <div class="field">
            <label for="new-pass">New passphrase</label>
            <input id="new-pass" type="password" autocomplete="new-password" bind:value={passphrase} />
          </div>
          <div class="field">
            <label for="new-pass2">Repeat it</label>
            <input id="new-pass2" type="password" autocomplete="new-password" bind:value={repeat} />
          </div>
          <p class="caption secondary">At least {MIN_PASSPHRASE} characters. {PASSPHRASE_WARNING}</p>
          {#if formError}<p class="caption error-text" role="alert">{formError}</p>{/if}
          <div class="row">
            <button class="primary" type="submit" disabled={working}>Change passphrase</button>
            <button type="button" onclick={() => { changing = false; resetForm(); }}>Cancel</button>
          </div>
        </form>
      {:else}
        <div class="row"><button onclick={() => (changing = true)}>Change passphrase…</button></div>
      {/if}
    </section>
  {/if}

  <div class="hairline"></div>

  <section aria-label="Account actions">
    <div class="row">
      <button onclick={() => acct.signOut()}>Sign out</button>
      {#if acct.unlocked}
        <button class="plain danger" onclick={() => acct.deleteAccount()}>Delete account…</button>
      {/if}
    </div>
  </section>
{/if}

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 560px;
  }
  h2 {
    font-size: 13px;
    font-weight: 600;
    margin: 0;
  }
  p {
    margin: 0;
  }
  .lede {
    font-size: 13px;
    max-width: 48ch;
  }
  .who {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 13px;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .field {
    display: grid;
    grid-template-columns: 130px minmax(0, 280px);
    align-items: center;
    gap: 8px;
  }
  .field label {
    color: var(--label-2);
  }
  .row,
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .check {
    gap: 6px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  li {
    display: flex;
    flex-direction: column;
  }
  .amber {
    color: var(--amber);
  }
  .error-text,
  .danger {
    color: var(--red);
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
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
