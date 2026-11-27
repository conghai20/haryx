// Account actions shared by the flyout and the dashboard. Destructive ones ask first with a native
// dialog, then answer the CLI's own y/N prompt on stdin.

import { ask, open } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import { describeError } from "./errors";
import { store } from "./store.svelte";
import { isInGroup } from "./groups";
import { haryxAccount } from "./haryx.svelte";
import { displayName } from "./tide";
import type { Account, Job } from "./types";

/** Group changes sync to the other machines at once, not on the 15-minute loop. */
async function groupChange(run: Promise<boolean>): Promise<boolean> {
  const ok = await run;
  if (ok) haryxAccount.syncSoon();
  return ok;
}

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
  /** Adds the account to `group`, keeping its other groups. */
  addToGroup: (a: Account, group: string) => groupChange(store.run("Adding to group", ["group", ref(a), group])),
  /** Takes the account out of `group` only. */
  removeFromGroup: (a: Account, group: string) =>
    groupChange(store.run("Removing from group", ["group", ref(a), "--remove", group])),
  /** Takes the account out of all its groups. */
  clearGroups: (a: Account) => groupChange(store.run("Removing from groups", ["group", ref(a), "--unset"])),
  /** Renames a group on every account in it (a name of another group merges the two). */
  renameGroup: (old: string, name: string) =>
    groupChange(store.run("Renaming group", ["group", "--rename", old, name])),

  /** Deletes a group after asking: every account in it leaves it; the accounts stay. */
  async deleteGroup(name: string) {
    const n = store.accounts.filter((a) => isInGroup(a, name)).length;
    const ok = await ask(
      `Removes the group from ${n} account${n === 1 ? "" : "s"}. The accounts stay, in their other groups or Ungrouped.`,
      { title: `Delete group ${name}?`, kind: "warning", okLabel: "Delete group", cancelLabel: "Cancel" },
    );
    if (ok) await groupChange(store.run("Deleting group", ["group", "--delete", name]));
  },

  /** The CLI takes one account per call; stops at the first that fails (its error is flashed). */
  async addToGroupMany(accounts: Account[], group: string): Promise<boolean> {
    let done = 0;
    for (const a of accounts) {
      if (!(await store.run("Setting group", ["group", ref(a), group]))) {
        if (done > 0) haryxAccount.syncSoon();
        return false;
      }
      done += 1;
    }
    haryxAccount.syncSoon();
    store.flash(`Group ${group}: ${accounts.length} account${accounts.length === 1 ? "" : "s"}`);
    return true;
  },

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

  /** The inspector's Force Switch asks first, as on macOS; the flyout menu's does not. */
  async forceUse(a: Account) {
    const ok = await ask("Activates the stored credentials without first backing up the current login.", {
      title: `Force switch to ${displayName(a)}?`,
      kind: "warning",
      okLabel: "Force switch",
      cancelLabel: "Cancel",
    });
    if (ok) await actions.use(a, true);
  },

  move: (a: Account, slot: number) => store.run("Moving", ["move", ref(a), String(slot)]),
  swap: (a: Account, b: Account) => store.run("Swapping", ["swap", ref(a), ref(b)]),
  async map(a: Account, path: string) {
    await store.run("Mapping", ["map", ref(a), path]);
    await store.loadMappings();
  },
  /** Asks first: the folder's plain `haryx ai run` goes back to the active account. */
  async unmap(path: string) {
    const ok = await ask(`Plain haryx ai run in ${path} will use the active account again. The folder itself is not touched.`, {
      title: "Unmap this folder?",
      kind: "warning",
      okLabel: "Unmap",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    await store.run("Unmapping", ["unmap", path]);
    await store.loadMappings();
  },

  /** A folder from the system picker, or null when cancelled. */
  async chooseFolder(title: string): Promise<string | null> {
    try {
      const folder = await open({ directory: true, title });
      return typeof folder === "string" ? folder : null;
    } catch (e) {
      store.fail(e, "Couldn't choose a folder");
      return null;
    }
  },

  /** Claude in a picked folder, as `a` (null = the active account). */
  async runInFolder(a: Account | null) {
    const folder = await actions.chooseFolder("Run Claude here");
    if (!folder) return;
    if (a) await actions.runAs(a, folder);
    else await actions.terminal({ kind: "claude" }, folder);
  },

  addCurrent: () => store.run("Adding the current login", ["add"], { success: "Current Claude Code login added" }),

  terminal: async (job: Job, folder?: string | null) => {
    try {
      await api.openTerminal(job, store.prefs?.terminal, folder);
      await api.hideFlyout().catch(() => {});
    } catch (e) {
      store.fail(e, "Couldn't open a terminal");
    }
  },

  /** Sign in (again) in HaryX's own sign-in window; a terminal only when that cannot start. */
  async signIn(a: Account | null) {
    const slot = a ? a.number : null;
    try {
      await api.loginStart(slot, a ? `Sign in again: ${a.email}` : "Sign in new account");
    } catch (e) {
      const fallback = await ask(`The sign-in window could not start. ${describeError(e).text}`, {
        title: "Sign in",
        kind: "warning",
        okLabel: "Use a terminal instead",
        cancelLabel: "Cancel",
      });
      if (fallback) await actions.terminal({ kind: "login", slot });
    }
  },
  runAs: (a: Account, folder?: string | null) => actions.terminal({ kind: "run-as", slot: a.number }, folder),
  /** End a session and resume its conversation on `to` (null = best), in a terminal in its folder. */
  continueSession: (pid: number, to: number | null, folder: string) =>
    actions.terminal({ kind: "continue", pid, to }, folder),
};
