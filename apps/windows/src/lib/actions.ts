// Account actions shared by the flyout and the dashboard. Destructive ones ask first with a native
// dialog, then answer the CLI's own y/N prompt on stdin.

import { ask } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import { store } from "./store.svelte";
import { displayName } from "./tide";
import type { Account, Job } from "./types";

/** Slot numbers are unambiguous; emails and aliases can collide. */
const ref = (a: Account) => String(a.number);

export const actions = {
  use: (a: Account, force = false) =>
    store.run(`Switching to ${displayName(a)}`, ["switch", ref(a), "--json", ...(force ? ["--force"] : [])], {
      success: `Now using ${displayName(a)}`,
    }),

  next: (strategy?: "best" | "next-available") =>
    store.run("Switching", strategy ? ["switch", "--strategy", strategy, "--json"] : ["switch", "--json"], {
      success: "Switched",
    }),

  setDisabled: (a: Account, disabled: boolean) =>
    store.run(disabled ? "Disabling" : "Enabling", [disabled ? "disable" : "enable", ref(a)]),

  setAlias: (a: Account, alias: string | null) =>
    store.run("Setting alias", alias ? ["alias", ref(a), alias] : ["alias", ref(a), "--unset"]),

  setThreshold: (a: Account, pct: number | null) =>
    store.run("Setting threshold", pct == null ? ["threshold", ref(a), "--unset"] : ["threshold", ref(a), String(pct)]),

  setPriority: (a: Account, tier: number | null) =>
    store.run("Setting priority", tier == null ? ["priority", ref(a), "--unset"] : ["priority", ref(a), String(tier)]),

  async remove(a: Account) {
    const ok = await ask(`Remove ${a.email} from HaryX? Its saved login is deleted.`, {
      title: "Remove account",
      kind: "warning",
      okLabel: "Remove",
      cancelLabel: "Cancel",
    });
    if (ok) await store.run("Removing", ["remove", ref(a)], { stdin: "y\n", success: `Removed ${a.email}` });
  },

  addCurrent: () => store.run("Adding the current login", ["add"], { success: "Current Claude Code login added" }),

  terminal: async (job: Job, folder?: string | null) => {
    try {
      await api.openTerminal(job, store.prefs?.terminal, folder);
      await api.hideFlyout().catch(() => {});
    } catch (e) {
      store.flash(String(e), true);
    }
  },

  signIn: (a: Account | null) => actions.terminal({ kind: "login", slot: a ? a.number : null }),
  runAs: (a: Account, folder?: string | null) => actions.terminal({ kind: "run-as", slot: a.number }, folder),
  /** End a session and resume its conversation on `to` (null = best), in a terminal in its folder. */
  continueSession: (pid: number, to: number | null, folder: string) =>
    actions.terminal({ kind: "continue", pid, to }, folder),
};
