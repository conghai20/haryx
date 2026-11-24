// Calls into the Rust backend. Every CLI call goes through `haryx` (allowlisted verbs, no shell).

import { invoke } from "@tauri-apps/api/core";
import type { Job, Prefs, RunResult, Snapshot, TerminalId } from "./types";

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  refresh: () => invoke<void>("refresh"),
  haryx: (args: string[], stdin?: string) => invoke<RunResult>("haryx", { args, stdin: stdin ?? null }),
  json: <T>(args: string[]) => invoke<T>("haryx_json", { args }),
  setPrefs: (prefs: Prefs) => invoke<void>("set_prefs", { prefs }),
  autoStart: () => invoke<void>("auto_start"),
  autoStop: () => invoke<void>("auto_stop"),
  openTerminal: (job: Job, terminal?: TerminalId | null, folder?: string | null) =>
    invoke<void>("open_terminal", { job, terminal: terminal ?? null, folder: folder ?? null }),
  openDashboard: (pane?: string) => invoke<void>("open_dashboard", { pane: pane ?? null }),
  hideFlyout: () => invoke<void>("hide_flyout"),
  quit: () => invoke<void>("quit"),
};
