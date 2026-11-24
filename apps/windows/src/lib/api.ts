// Calls into the Rust backend. Every CLI call goes through `haryx` (allowlisted verbs, no shell).

import { invoke } from "@tauri-apps/api/core";
import type { Job, Prefs, RunResult, Snapshot, TerminalId, UpdateMode } from "./types";

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  refresh: () => invoke<void>("refresh"),
  haryx: (args: string[], stdin?: string, passphrase?: string) =>
    invoke<RunResult>("haryx", { args, stdin: stdin ?? null, passphrase: passphrase ?? null }),
  json: <T>(args: string[]) => invoke<T>("haryx_json", { args }),
  setPrefs: (prefs: Prefs) => invoke<void>("set_prefs", { prefs }),
  autoStart: () => invoke<void>("auto_start"),
  autoStop: () => invoke<void>("auto_stop"),
  openTerminal: (job: Job, terminal?: TerminalId | null, folder?: string | null) =>
    invoke<void>("open_terminal", { job, terminal: terminal ?? null, folder: folder ?? null }),
  openDashboard: (pane?: string) => invoke<void>("open_dashboard", { pane: pane ?? null }),
  hideFlyout: () => invoke<void>("hide_flyout"),
  quit: () => invoke<void>("quit"),
  updateCheck: (refresh: boolean) => invoke<void>("update_check", { refresh }),
  updateDownload: () => invoke<void>("update_download"),
  updateRestart: () => invoke<void>("update_restart"),
  openLink: (url: string) => invoke<void>("open_link", { url }),
  updateSet: (opts: { mode?: UpdateMode; skip?: string }) =>
    invoke<void>("update_set", { mode: opts.mode ?? null, skip: opts.skip ?? null }),
};
