// The CLI's --json shapes the UI reads (contracts: packages/contracts/). Every optional field may be
// missing; a newer CLI may add fields.

export interface UsageWindow {
  pct?: number | null;
  resetsAt?: string | null;
  countdown?: string | null;
  clock?: string | null;
  expectedPct?: number | null;
  aheadOfPace?: boolean | null;
  projectedExhaustionAt?: string | null;
  willLastToReset?: boolean | null;
}

export interface Account {
  number: number;
  email: string;
  organizationName?: string | null;
  organizationUuid?: string | null;
  active: boolean;
  usageStatus: string;
  usage?: { fiveHour?: UsageWindow | null; sevenDay?: UsageWindow | null } | null;
  alias?: string | null;
  /** `haryx group`: the first group, absent when ungrouped. Older CLIs send only this. */
  group?: string | null;
  /** Every group the account is in, in the order added; absent when ungrouped. */
  groups?: string[] | null;
  disabled?: boolean | null;
  threshold?: number | null;
  loginExpiresAt?: string | null;
  plan?: { label?: string | null; expired?: boolean | null } | null;
  usageAgeSeconds?: number | null;
  /** The reading was taken by another of your machines (usage share through HaryX sync). */
  usageFromOtherMachine?: boolean | null;
  usageError?: string | null;
}

export interface AccountList {
  schemaVersion: number;
  activeAccountNumber?: number | null;
  accounts: Account[];
}

export interface Session {
  pid: number;
  account: number | null;
  email: string;
  profile: "default" | "session" | string;
  sessionId: string;
  cwd: string;
  startedAt: number;
  kind: string;
  entrypoint: string;
  status: string | null;
}

export interface SessionList {
  sessions: Session[];
  unreadable: number;
}

export interface AutoEvent {
  kind: string;
  ts: string | null;
  summary: string;
  severity: "info" | "notable" | "problem";
  notificationTitle: string | null;
  fromAccount?: number | null;
}

export type TerminalId = "windows-terminal" | "pwsh" | "powershell" | "cmd" | "git-bash" | "warp";

export interface Prefs {
  refreshSeconds: number;
  terminal: TerminalId | null;
  claudeFolder: string | null;
  haryxPath: string | null;
  autoSwitchOnLaunch: boolean;
  autoSwitchDryRun: boolean;
  notifySwitched: boolean;
  notifyUsage: boolean;
  notifyLogin: boolean;
  notifyReset: boolean;
  notifySession: boolean;
  notifyAutoSwitch: boolean;
  trayMascot: boolean;
  /** Mascot choices by identity key (`email|org`); unlisted accounts get a hashed species. */
  mascotStyles: Record<string, MascotStyle>;
  launchAtLogin: boolean;
  theme: Theme;
  updateMode: UpdateMode;
  skippedVersion: string | null;
  lastUpdateCheck: number | null;
  weeklySummary: boolean;
  lastWeeklySummary: number | null;
  /** Sync with the HaryX account when signed in and unlocked. This PC only; never synced. */
  syncEnabled: boolean;
}

/** The app's look; "system" follows Windows. */
export type Theme = "system" | "light" | "dark";

/** One account's mascot choice (macOS `MascotStyle`); `hue` 0–1, absent = the species' colour. */
export interface MascotStyle {
  species: string;
  hue?: number | null;
}

export type UpdateMode = "automatic" | "notify" | "manual";

/** `haryx upgrade --check --json` (contract: packages/contracts/haryx/upgrade-check.json). */
export interface UpgradeCheck {
  current: string;
  latest: string | null;
  available: boolean;
  published_at: string | null;
  notes: string | null;
  html_url: string | null;
  install: string;
  error: string | null;
}

export type UpdatePhase = "idle" | "checking" | "downloading" | "ready" | "failed";

/** The backend's update state (`updater::snapshot_json`). */
export interface UpdateInfo {
  state: {
    check: UpgradeCheck | null;
    phase: UpdatePhase;
    error: string | null;
    readyVersion: string | null;
    appVersion: string;
  };
  available: string | null;
  mode: UpdateMode;
  skipped: string | null;
  lastCheck: number | null;
}

export interface Snapshot {
  accounts: AccountList | null;
  error: string | null;
  sessions: SessionList;
  prefs: Prefs;
  auto: { running: boolean; log: AutoEvent[] };
  terminals: { id: TerminalId; title: string }[];
  cliPath: string | null;
  storeRoot: string | null;
  platform: string;
  update: UpdateInfo;
}

/** A folder whose plain `haryx run` uses an account (`haryx map`; backend `models::Mapping`). */
export interface Mapping {
  path: string;
  email: string;
  organizationUuid: string;
}

export type Job =
  | { kind: "shell" }
  | { kind: "claude" }
  | { kind: "run-as"; slot: number }
  | { kind: "run-mapped" }
  | { kind: "login"; slot: number | null }
  | { kind: "continue"; pid: number; to: number | null }
  | { kind: "install-claude" };

/** `haryx ai doctor --json`: whether Claude Code is installed and signed in, and how to fix it. */
export interface ClaudeSetup {
  ready: boolean;
  claude: string | null;
  email: string | null;
  installCommand: string;
  /** First is the one to fix first (signing in needs Claude Code installed). */
  problems: { code: "claude_missing" | "not_signed_in" | string; message: string; fix: string }[];
}

/** One in-app sign-in run (`login::View` in the backend). */
export interface LoginView {
  id: number;
  title: string;
  state: "running" | "succeeded" | "failed" | "cancelled";
  message: string | null;
  /** The run's output, escape sequences removed. */
  text: string;
  url: string | null;
  awaitingCode: boolean;
  awaitingAnswer: boolean;
}

export interface RunResult {
  ok: boolean;
  status: number;
  stdout: string;
  message: string;
}

// HaryX account and sync (contracts: packages/contracts/haryx/whoami.json, login-events.json,
// sync-status.json, sync-result.json).

export interface HaryxUser {
  id: string;
  email: string;
  provider: "github" | "google" | string;
}

/** `haryx whoami --json` and `haryx logout --json`. */
export interface Whoami {
  signedIn: boolean;
  user: HaryxUser | null;
}

/** One `haryx login --json` line (backend `account::LoginEvent`); `cancelled` is the app's own. */
export type AccountLoginEvent =
  | { event: "browser"; url: string; opened: boolean }
  | { event: "signed_in"; user: HaryxUser }
  | { event: "error"; code: string; message: string }
  | { event: "cancelled" };

/** A Claude account sync knows of that has no login on this PC. */
export interface OtherMachineAccount {
  email: string;
  organizationName?: string | null;
  alias?: string | null;
}

/** A value a newer write replaced. `lost` is null when the losing side had it unset. */
export interface Overwrite {
  at: number;
  kind: string;
  label: string;
  kept: "local" | "remote" | string;
  lost: unknown;
}

/** `haryx sync status --json`: local only, no network. */
export interface SyncStatus {
  signedIn: boolean;
  unlocked: boolean;
  lastSyncAt: string | null;
  otherMachineAccounts: OtherMachineAccount[];
  overwritten: Overwrite[];
}

/** The `{"ok": false, "code", "message"}` failure every account and sync command can print. */
export interface CliFailure {
  ok: false;
  code: string;
  message: string;
}

/** `haryx sync --json`. */
export type SyncResult =
  | {
      ok: true;
      pushed: number;
      pulled: number;
      changed: string[];
      overwritten: Overwrite[];
      otherMachineAccounts: OtherMachineAccount[];
      skipped: unknown[];
    }
  | CliFailure;

/** The backend's sync state (`sync::Syncer::info`). */
export interface SyncInfo {
  running: boolean;
  last: SyncResult | null;
}
