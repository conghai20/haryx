<script lang="ts">
  // HaryX › HaryX Account: sign in, consent to sync, unlock it with the passphrase, then sync
  // settings, preferences and account setup between machines; then privacy: showing this PC on
  // HaryX web, withdrawing a consent, exporting. Claude logins never leave this PC.
  // Mirrors macOS HaryXAccountPane.
  import { ask, save } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import ConsentDialog from "../components/ConsentDialog.svelte";
  import MenuButton from "../components/MenuButton.svelte";
  import {
    MIN_PASSPHRASE,
    PASSPHRASE_WARNING,
    SIGN_IN_PROVIDERS,
    overwriteLine,
    passphraseProblem,
    providerName,
    syncedAgo,
  } from "../lib/account";
  import { api } from "../lib/api";
  import { CONSENT_COPY, HARYX_WEB, POLICY_VERSION, PRIVACY_POLICY } from "../lib/consent";
  import { haryxAccount as acct } from "../lib/haryx.svelte";
  import { store } from "../lib/store.svelte";
  import type { ConsentPurpose } from "../lib/types";

  let passphrase = $state("");
  let repeat = $state("");
  let formError = $state<string | null>(null);
  let working = $state(false);
  let changing = $state(false);

  let creating = $derived(acct.hasPassphrase === false);
  let user = $derived(acct.whoami?.user ?? null);
  let overwritten = $derived((acct.last?.ok ? acct.last.overwritten : acct.status?.overwritten) ?? []);
  let privacyNote = $state<string | null>(null);
  let privacyError = $state<string | null>(null);
  let signInSections = $derived([
    { heading: null, items: SIGN_IN_PROVIDERS.map((p) => ({ label: p.menu, onpick: () => acct.startSignIn(p.id) })) },
  ]);

  function openLink(url: string) {
    api.openLink(url).catch((e) => store.flash(String(e), true));
  }

  async function withdraw(purpose: ConsentPurpose) {
    const copy = CONSENT_COPY[purpose];
    const ok = await ask(copy.withdrawMessage, {
      title: copy.withdrawTitle,
      kind: "warning",
      okLabel: copy.withdrawButton,
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    privacyError = await acct.setConsent(purpose, false);
    privacyNote = privacyError ? null : "Withdrawn. Its data is gone from HaryX's servers.";
  }

  function toggleMachines(e: Event & { currentTarget: HTMLInputElement }) {
    const on = e.currentTarget.checked;
    // The box follows the consent, not the click: the dialog or the confirmation decides.
    e.currentTarget.checked = acct.machinesConsent === true;
    if (on) acct.consentRequest = "machines";
    else void withdraw("machines");
  }

  async function exportData() {
    const path = await save({
      defaultPath: `haryx-export-${new Date().toISOString().slice(0, 10)}.json`,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path) return;
    privacyError = await acct.exportTo(path);
    privacyNote = privacyError
      ? null
      : "Saved. Synced data in it stays encrypted: only your sync passphrase opens it.";
  }

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
    {#if acct.signedOutFromWeb}
      <div class="banner" role="status">
        <strong>Signed out of HaryX from the web</strong>
        <span class="secondary">Sign in again to keep syncing. Everything on this PC keeps working.</span>
      </div>
    {/if}
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
        <MenuButton sections={signInSections} align="start" label={acct.signIn.phase === "error" ? "Try again" : "Sign in…"} />
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
    <div class="row"><button class="link" onclick={() => openLink(HARYX_WEB)}>Open HaryX on the web</button></div>
  </section>

  <div class="hairline"></div>

  {#if acct.needsSyncConsent}
    <section aria-labelledby="consent-title">
      <h2 id="consent-title">Sync</h2>
      {#if acct.problem?.consent && acct.syncConsent !== false}
        <p class="caption amber" role="status">{acct.problem.text}</p>
      {/if}
      <p class="secondary">
        Sync keeps settings, app preferences and account setup the same on your PCs and Macs. HaryX asks for your
        consent first.
      </p>
      <div class="row"><button class="primary" onclick={() => (acct.consentRequest = "sync")}>Turn on sync…</button></div>
    </section>
  {:else if !acct.unlocked}
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

  <section aria-labelledby="privacy-title">
    <h2 id="privacy-title">Privacy</h2>
    <label class="check">
      <input
        type="checkbox"
        checked={acct.machinesConsent === true}
        disabled={acct.machinesConsent == null || acct.working}
        onchange={toggleMachines}
      />
      Show this PC on HaryX web
    </label>
    {#if acct.syncConsent === true}
      <div class="row">
        <span>Sync consent given</span>
        <button disabled={acct.working} onclick={() => withdraw("sync")}>Withdraw…</button>
      </div>
    {/if}
    <div class="row">
      <span>A copy of everything HaryX keeps about your account</span>
      <button disabled={acct.working} onclick={exportData}>Export…</button>
    </div>
    {#if privacyNote}<p class="caption secondary" role="status">{privacyNote}</p>{/if}
    {#if privacyError}<p class="caption error-text" role="alert">{privacyError}</p>{/if}
    {#if acct.machinesConsent == null}
      <p class="caption tertiary">HaryX could not read your consents. Check your connection.</p>
    {/if}
    <div class="row">
      <button class="link caption" onclick={() => openLink(PRIVACY_POLICY)}>Privacy policy, version {POLICY_VERSION}</button>
    </div>
  </section>

  <div class="hairline"></div>

  <section aria-label="Account actions">
    <div class="row">
      <button onclick={() => acct.signOut()}>Sign out</button>
      <button class="plain danger" onclick={() => acct.deleteAccount()}>Delete account…</button>
    </div>
    <p class="caption tertiary">Signing out forgets this PC's sync key. Your Claude accounts on this PC are not touched.</p>
  </section>
{/if}

{#if acct.consentRequest}
  <ConsentDialog purpose={acct.consentRequest} onclose={() => (acct.consentRequest = null)} />
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
  .banner {
    display: flex;
    flex-direction: column;
    gap: 2px;
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
