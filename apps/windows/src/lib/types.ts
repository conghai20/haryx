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
  disabled?: boolean | null;
  threshold?: number | null;
  loginExpiresAt?: string | null;
  plan?: { label?: string | null; expired?: boolean | null } | null;
  usageAgeSeconds?: number | null;
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

export type TerminalId = "windows-terminal" | "pwsh" | "powershell" | "cmd" | "git-bash";

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
