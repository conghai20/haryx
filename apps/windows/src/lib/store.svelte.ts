// App state for both windows: filled from the backend's snapshot, then kept current by its events
// (accounts, sessions, auto-event, prefs). Actions report through `flash`.

import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { ordered } from "./tide";
import type { Account, AccountList, AutoEvent, Prefs, SessionList, Snapshot } from "./types";

const AUTO_LOG_MAX = 200;
const FLASH_MS = 6000;

class Store {
  list = $state<AccountList | null>(null);
  error = $state<string | null>(null);
  sessions = $state<SessionList>({ sessions: [], unreadable: 0 });
  prefs = $state<Prefs | null>(null);
  autoRunning = $state(false);
  autoLog = $state<AutoEvent[]>([]);
  terminals = $state<Snapshot["terminals"]>([]);
  cliPath = $state<string | null>(null);
  storeRoot = $state<string | null>(null);
  busy = $state<string | null>(null);
  message = $state<{ text: string; error: boolean } | null>(null);
  now = $state(Date.now());

  accounts = $derived<Account[]>(ordered(this.list?.accounts ?? []));
  active = $derived<Account | null>(this.accounts.find((a) => a.active) ?? null);
  others = $derived<Account[]>(this.accounts.filter((a) => !a.active));
  lastAutoEvent = $derived<AutoEvent | null>(
    [...this.autoLog].reverse().find((e) => e.kind !== "poll") ?? null,
  );

  #flashTimer: ReturnType<typeof setTimeout> | null = null;

  async init() {
    this.apply(await api.snapshot());
    await listen<{ list: AccountList | null; error: string | null }>("accounts", (e) => {
      if (e.payload.list) this.list = e.payload.list;
      this.error = e.payload.error;
      this.now = Date.now();
    });
    await listen<SessionList>("sessions", (e) => (this.sessions = e.payload));
    await listen<AutoEvent>("auto-event", (e) => {
      this.autoLog = [...this.autoLog, e.payload].slice(-AUTO_LOG_MAX);
      if (e.payload.kind === "started") this.autoRunning = true;
      if (e.payload.kind === "stopped" || (e.payload.kind === "error" && e.payload.notificationTitle === "Auto-switch stopped"))
        this.autoRunning = false;
    });
    await listen<Prefs>("prefs", (e) => (this.prefs = e.payload));
    setInterval(() => (this.now = Date.now()), 30_000);
  }

  apply(s: Snapshot) {
    this.list = s.accounts;
    this.error = s.error;
    this.sessions = s.sessions;
    this.prefs = s.prefs;
    this.autoRunning = s.auto.running;
    this.autoLog = s.auto.log;
    this.terminals = s.terminals;
    this.cliPath = s.cliPath;
    this.storeRoot = s.storeRoot;
  }

  async reload() {
    this.apply(await api.snapshot());
  }

  flash(text: string, error = false) {
    this.message = { text, error };
    if (this.#flashTimer) clearTimeout(this.#flashTimer);
    this.#flashTimer = setTimeout(() => (this.message = null), FLASH_MS);
  }

  /** Run a haryx command with a busy label; flash the CLI's last line or the given success text. */
  async run(
    label: string,
    args: string[],
    opts: { stdin?: string; success?: string; passphrase?: string } = {},
  ): Promise<boolean> {
    this.busy = label;
    try {
      const r = await api.haryx(args, opts.stdin, opts.passphrase);
      if (r.ok) this.flash(opts.success ?? (r.message || `${label}: done`));
      else this.flash(r.message, true);
      return r.ok;
    } catch (e) {
      this.flash(String(e), true);
      return false;
    } finally {
      this.busy = null;
    }
  }

  runningFor(a: Account): number {
    return this.sessions.sessions.filter((s) => s.account === a.number).length;
  }

  async savePrefs(patch: Partial<Prefs>) {
    if (!this.prefs) return;
    const next = { ...this.prefs, ...patch };
    try {
      await api.setPrefs(next);
      this.prefs = next;
      await this.reload();
    } catch (e) {
      this.flash(String(e), true);
    }
  }

  async toggleAuto() {
    try {
      if (this.autoRunning) await api.autoStop();
      else await api.autoStart();
      this.autoRunning = !this.autoRunning;
    } catch (e) {
      this.flash(String(e), true);
    }
  }
}

export const store = new Store();
