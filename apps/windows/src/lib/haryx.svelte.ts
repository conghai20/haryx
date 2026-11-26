// HaryX account state for the dashboard: who is signed in, whether sync is unlocked, the running
// sign-in and the last sync. Everything comes from the CLI; the backend runs automatic syncs and
// reports them as `sync` events.

import { listen } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import { DELETE_CONFIRM, isFailure, parseCli, syncProblem, type SignInProvider, type SyncProblem } from "./account";
import { store } from "./store.svelte";
import type { AccountLoginEvent, CliFailure, OtherMachineAccount, SyncResult, SyncStatus, Whoami } from "./types";

/** After a group change: short, but long enough that a change made of several CLI calls syncs once. */
const SYNC_SOON_MS = 1000;

type SignIn =
  | { phase: "idle" }
  | { phase: "waiting"; hasLink: boolean; browserOpened: boolean }
  | { phase: "error"; message: string };

const TIMEOUT_MESSAGE = "No sign-in arrived within 5 minutes.";

class HaryxAccount {
  whoami = $state<Whoami | null>(null);
  status = $state<SyncStatus | null>(null);
  /** Read from the server when locked: set a passphrase (false) or enter it (true). */
  hasPassphrase = $state<boolean | null>(null);
  loadError = $state<string | null>(null);
  signIn = $state<SignIn>({ phase: "idle" });
  /** The provider of the sign-in in progress, or of the last one. */
  signInProvider = $state<SignInProvider>("github");
  syncing = $state(false);
  last = $state<SyncResult | null>(null);
  /** A sync said the key changed or is locked, before `sync status` caught up. */
  #needsUnlock = $state(false);
  #ready: Promise<void> | null = null;

  signedIn = $derived(this.whoami?.signedIn === true && this.whoami.user != null);
  unlocked = $derived(this.signedIn && this.status?.unlocked === true && !this.#needsUnlock);
  /** The last sync's problem, as quiet status text. */
  problem = $derived<SyncProblem | null>(this.last && isFailure(this.last) ? syncProblem(this.last) : null);
  otherMachines = $derived<OtherMachineAccount[]>(
    this.signedIn && this.last?.ok ? this.last.otherMachineAccounts : (this.status?.otherMachineAccounts ?? []),
  );

  /** Listen once per window and read the current state. */
  init(): Promise<void> {
    this.#ready ??= this.#start();
    return this.#ready;
  }

  async #start() {
    await listen<AccountLoginEvent>("account-login", (e) => void this.#onLogin(e.payload));
    await listen<{ running: boolean; last: SyncResult | null }>("sync", (e) => this.#onSync(e.payload));
    try {
      const info = await api.syncInfo();
      this.syncing = info.running;
      this.last = info.last;
    } catch (e) {
      this.loadError = String(e);
    }
    await this.refresh();
  }

  async #json<T extends object>(args: string[], stdin?: string): Promise<T | CliFailure> {
    try {
      return parseCli<T>(await api.haryx(args, stdin));
    } catch (e) {
      return { ok: false, code: "local", message: String(e) };
    }
  }

  /** whoami and sync status (both local, no network). */
  async refresh() {
    const who = await this.#json<Whoami>(["whoami", "--json"]);
    if (isFailure(who)) {
      this.loadError = who.message;
      return;
    }
    this.loadError = null;
    this.whoami = who;
    if (!who.signedIn) {
      this.status = null;
      this.hasPassphrase = null;
      return;
    }
    const status = await this.#json<SyncStatus>(["sync", "status", "--json"]);
    if (!isFailure(status)) {
      this.status = status;
      if (status.unlocked) this.#needsUnlock = false;
    }
    if (!this.unlocked && this.hasPassphrase == null) await this.checkPassphrase();
  }

  /** Whether the account has a passphrase yet (network). */
  async checkPassphrase() {
    const r = await this.#json<{ ok: true; hasPassphrase: boolean }>(["sync", "has-passphrase", "--json"]);
    if (isFailure(r)) this.loadError = syncProblem(r).text;
    else {
      this.loadError = null;
      this.hasPassphrase = r.hasPassphrase;
    }
  }

  async startSignIn(provider: SignInProvider = "github") {
    this.signInProvider = provider;
    this.signIn = { phase: "waiting", hasLink: false, browserOpened: false };
    try {
      await api.accountLoginStart(provider);
    } catch (e) {
      this.signIn = { phase: "error", message: String(e) };
    }
  }

  async cancelSignIn() {
    await api.accountLoginCancel().catch((e) => store.flash(String(e), true));
  }

  async openSignInPage() {
    await api.accountLoginOpen().catch((e) => store.flash(String(e), true));
  }

  async #onLogin(e: AccountLoginEvent) {
    switch (e.event) {
      case "browser":
        this.signIn = { phase: "waiting", hasLink: true, browserOpened: e.opened };
        break;
      case "signed_in":
        this.signIn = { phase: "idle" };
        this.hasPassphrase = null;
        await this.refresh();
        break;
      case "error":
        this.signIn = { phase: "error", message: e.code === "timeout" ? TIMEOUT_MESSAGE : e.message };
        break;
      case "cancelled":
        this.signIn = { phase: "idle" };
        break;
    }
  }

  #onSync(info: { running: boolean; last: SyncResult | null }) {
    this.syncing = info.running;
    if (info.running) return;
    this.last = info.last;
    if (info.last && isFailure(info.last)) {
      const p = syncProblem(info.last);
      if (p.unlock) this.#needsUnlock = true;
      if (p.signedOut || p.unlock) void this.refresh();
    } else void this.refresh();
  }

  /** Set (first machine) or enter the passphrase. Returns the problem to show, or null. */
  async unlock(passphrase: string): Promise<string | null> {
    const r = await this.#json<{ ok: true; created: boolean }>(
      ["sync", "unlock", "--passphrase-stdin", "--json"],
      `${passphrase}\n`,
    );
    if (isFailure(r)) return syncProblem(r).text;
    this.#needsUnlock = false;
    this.hasPassphrase = true;
    await this.refresh();
    void this.syncNow();
    return null;
  }

  async changePassphrase(passphrase: string): Promise<string | null> {
    const r = await this.#json<{ ok: true }>(["sync", "passphrase", "--passphrase-stdin", "--json"], `${passphrase}\n`);
    if (isFailure(r)) return syncProblem(r).text;
    store.flash("Sync passphrase changed");
    return null;
  }

  #syncSoonTimer: ReturnType<typeof setTimeout> | null = null;

  /**
   * A synced account setting changed here (a group added, removed, renamed or deleted): sync
   * shortly, rather than on the 15-minute loop. Several changes in a row (New Group with many
   * accounts) sync once. Skipped quietly when sync is off, signed out or locked.
   */
  syncSoon() {
    if (this.#syncSoonTimer) clearTimeout(this.#syncSoonTimer);
    this.#syncSoonTimer = setTimeout(() => {
      this.#syncSoonTimer = null;
      api
        .syncAfterChange()
        .then((r) => r && this.#onSync({ running: false, last: r }))
        .catch(() => {
          // An automatic sync fails quietly; the 15-minute loop retries.
        });
    }, SYNC_SOON_MS);
  }

  async syncNow() {
    try {
      const r = await api.syncNow();
      if (r) this.#onSync({ running: false, last: r });
    } catch (e) {
      this.#onSync({ running: false, last: { ok: false, code: "local", message: String(e) } });
    }
  }

  async signOut() {
    const r = await this.#json<Whoami>(["logout", "--json"]);
    if (isFailure(r)) store.flash(r.message, true);
    this.last = null;
    this.hasPassphrase = null;
    await this.refresh();
  }

  async deleteAccount() {
    const ok = await ask(DELETE_CONFIRM, {
      title: "Delete HaryX account",
      kind: "warning",
      okLabel: "Delete Account",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    const r = await this.#json<{ ok: true }>(["account", "delete", "--yes", "--json"]);
    if (isFailure(r)) {
      store.flash(r.message, true);
      return;
    }
    store.flash("HaryX account deleted");
    this.last = null;
    this.hasPassphrase = null;
    await this.refresh();
  }

  /** "Sign in here": the usual new-account Claude sign-in. The backend syncs once it finishes, and
   *  sync applies the account's alias, priority and the rest by email and organization. */
  async signInHere(a: OtherMachineAccount) {
    try {
      await api.loginStart(null, `Sign In: ${a.email}`);
    } catch (e) {
      store.flash(String(e), true);
    }
  }

  async forget(a: OtherMachineAccount) {
    const ok = await ask(`Forget ${a.email} on every machine? Machines where it is signed in keep it.`, {
      title: "Forget account",
      kind: "warning",
      okLabel: "Forget",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    const r = await this.#json<{ ok: true; removed: number }>(["sync", "forget", a.email, "--json"]);
    if (isFailure(r)) {
      store.flash(r.message, true);
      return;
    }
    store.flash(`Forgot ${a.email}`);
    this.last = null;
    await this.refresh();
  }
}

export const haryxAccount = new HaryxAccount();
