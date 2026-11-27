// App state for both windows: filled from the backend's snapshot, then kept current by its events
// (accounts, sessions, auto-event, prefs). Actions report through `flash`.

import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { groupOrdered } from "./groups";
import { stuckSessions } from "./handoff";
import { SPECIES, identityKey, mood, resolve, type Mood, type Style } from "./mascot";
import { describeError } from "./errors";
import { ordered } from "./tide";
import type { QuotaReport } from "./usage";
import type { Account, AccountList, AutoEvent, ClaudeSetup, Mapping, MascotStyle, Prefs, SessionList, Snapshot, UpdateInfo } from "./types";

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
  update = $state<UpdateInfo | null>(null);
  /** `haryx ai doctor`: null until read. */
  claudeSetup = $state<ClaudeSetup | null>(null);
  /** Folders `haryx map` ties to an account, sorted by path. */
  mappings = $state<Mapping[]>([]);
  /** An account the dashboard should select (the flyout's Info); Accounts takes and clears it. */
  revealSlot = $state<number | null>(null);
  busy = $state<string | null>(null);
  /** The banner: a success line clears itself; an error stays until it is dismissed or replaced. */
  message = $state<{ text: string; error: boolean; detail?: string } | null>(null);
  now = $state(Date.now());
  /** Each account's weekly readings over the last 7 days ([epoch s, %]), for its Week Tide. Kept only
   *  while usage history is on; empty otherwise, and the tide draws its average instead. */
  weekReadings = $state<Record<number, [number, number][]>>({});

  accounts = $derived<Account[]>(ordered(this.list?.accounts ?? []));
  active = $derived<Account | null>(this.accounts.find((a) => a.active) ?? null);
  /** The other accounts clustered by group, the active account's first, as the CLI's automatic picks prefer it. */
  others = $derived<Account[]>(groupOrdered(this.accounts.filter((a) => !a.active), this.active));
  stuck = $derived(stuckSessions(this.sessions.sessions, this.accounts));
  /** Every account's mascot, by identity key: the same resolution the tray uses. */
  mascots = $derived<Record<string, Style>>(
    resolve(this.prefs?.mascotStyles ?? {}, this.accounts.map(identityKey)),
  );
  lastAutoEvent = $derived<AutoEvent | null>(
    [...this.autoLog].reverse().find((e) => e.kind !== "poll") ?? null,
  );

  #flashTimer: ReturnType<typeof setTimeout> | null = null;

  async init() {
    this.apply(await api.snapshot());
    void this.checkClaudeSetup();
    void this.loadMappings();
    void this.loadWeekReadings();
    await listen<{ list: AccountList | null; error: string | null }>("accounts", (e) => {
      if (e.payload.list) this.list = e.payload.list;
      this.error = e.payload.error;
      this.now = Date.now();
      // Every haryx command refreshes the accounts, so a map or unmap made elsewhere shows up too.
      void this.loadMappings();
      void this.loadWeekReadings();
    });
    await listen<SessionList>("sessions", (e) => {
      this.sessions = e.payload;
    });
    await listen<AutoEvent>("auto-event", (e) => {
      this.autoLog = [...this.autoLog, e.payload].slice(-AUTO_LOG_MAX);
      if (e.payload.kind === "started") this.autoRunning = true;
      if (e.payload.kind === "stopped" || (e.payload.kind === "error" && e.payload.notificationTitle === "Auto-switch stopped"))
        this.autoRunning = false;
    });
    await listen<Prefs>("prefs", (e) => (this.prefs = e.payload));
    await listen<UpdateInfo>("update", (e) => (this.update = e.payload));
    setInterval(() => (this.now = Date.now()), 30_000);
  }

  /** Re-read whether Claude Code is installed and signed in. A failed read keeps the last answer. */
  async checkClaudeSetup() {
    try {
      this.claudeSetup = await api.json<ClaudeSetup>(["ai", "doctor", "--json"]);
    } catch {
      // An older haryx without `ai doctor`, or a CLI error the accounts list already reports.
    }
  }

  /** `haryx ai usage quota --since 7d`: a file read. A failed read keeps the last readings. */
  async loadWeekReadings() {
    try {
      const quota = await api.json<QuotaReport>(["ai", "usage", "quota", "--since", "7d", "--json"]);
      const byAccount: Record<number, [number, number][]> = {};
      for (const r of quota.readings) {
        if (r.sevenDay == null) continue;
        (byAccount[r.account] ??= []).push([r.at, r.sevenDay]);
      }
      this.weekReadings = byAccount;
    } catch {
      // An older haryx without `ai usage`: the tides draw averages.
    }
  }

  async loadMappings() {
    try {
      this.mappings = await api.mappings();
    } catch (e) {
      this.fail(e, "Couldn't read folder mappings");
    }
  }

  /** The account a mapping names, matched by identity (slots move; email and org do not). */
  accountFor(m: Mapping): Account | null {
    return this.accounts.find((a) => identityKey(a) === `${m.email}|${m.organizationUuid}`) ?? null;
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
    this.update = s.update;
  }

  async reload() {
    this.apply(await api.snapshot());
  }

  /** Show a line in the banner. Success lines clear after a few seconds; errors stay until dismissed. */
  flash(text: string, error = false, detail?: string) {
    this.message = { text, error, detail };
    if (this.#flashTimer) clearTimeout(this.#flashTimer);
    this.#flashTimer = error ? null : setTimeout(() => (this.message = null), FLASH_MS);
  }

  /** Show what failed in plain words; the raw error goes in the banner's detail. */
  fail(e: unknown, what?: string) {
    const { text, detail } = describeError(e, what);
    this.flash(text, true, detail);
  }

  dismiss() {
    if (this.#flashTimer) clearTimeout(this.#flashTimer);
    this.#flashTimer = null;
    this.message = null;
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
      else this.fail(r.message, `${label} failed`);
      return r.ok;
    } catch (e) {
      this.fail(e, `${label} failed`);
      return false;
    } finally {
      this.busy = null;
    }
  }

  runningFor(a: Account): number {
    return this.sessions.sessions.filter((s) => s.account === a.number).length;
  }

  styleFor(a: Account): Style {
    return this.mascots[identityKey(a)] ?? { species: SPECIES[0]!, hue: null };
  }

  moodFor(a: Account): Mood {
    return mood(a, this.runningFor(a) > 0);
  }

  /** Save an account's mascot choice; `null` returns it to the hashed default. */
  async setMascotStyle(a: Account, style: MascotStyle | null) {
    const { [identityKey(a)]: _, ...rest } = this.prefs?.mascotStyles ?? {};
    await this.savePrefs({ mascotStyles: style ? { ...rest, [identityKey(a)]: style } : rest });
  }

  async savePrefs(patch: Partial<Prefs>) {
    if (!this.prefs) return;
    const next = { ...this.prefs, ...patch };
    try {
      await api.setPrefs(next);
      this.prefs = next;
      await this.reload();
    } catch (e) {
      this.fail(e, "Couldn't save the setting");
    }
  }

  async toggleAuto() {
    try {
      if (this.autoRunning) await api.autoStop();
      else await api.autoStart();
      this.autoRunning = !this.autoRunning;
    } catch (e) {
      this.fail(e, this.autoRunning ? "Couldn't stop auto-switch" : "Couldn't start auto-switch");
    }
  }
}

export const store = new Store();
