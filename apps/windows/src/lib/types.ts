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
  launchAtLogin: boolean;
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
}

export type Job =
  | { kind: "shell" }
  | { kind: "claude" }
  | { kind: "run-as"; slot: number }
  | { kind: "run-mapped" }
  | { kind: "login"; slot: number | null };

export interface RunResult {
  ok: boolean;
  status: number;
  stdout: string;
  message: string;
}
