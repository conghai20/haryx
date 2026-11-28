// The signed-in app's shared state: the consents, the decoded synced data once the sync lock is
// open, and the machine names. The hero panel and every section read the same copy, so a lock,
// an unlock or a consent change shows everywhere at once. Signed out is the shell's: a 401
// anywhere drops the whole app to the sign-in screen.
import { api } from "./auth.svelte";
import { ApiError } from "./api";
import { isGranted } from "./consents";
import { decodeSynced, type Synced } from "./synced";
import { syncLock } from "./synclock";
import type { Consent, HaryxMachine } from "./types";

export type VaultState = "loading" | "error" | "syncOff" | "locked" | "unlocking" | "decoding" | "decodeFailed" | "ready";

class Vault {
  consents = $state.raw<Consent[] | null>(null);
  consentsFailed = $state(false);
  synced = $state.raw<Synced | null>(null);
  decodeFailed = $state(false);
  machines = $state.raw<HaryxMachine[]>([]);
  machinesFailed = $state(false);

  state: VaultState = $derived.by(() => {
    if (this.consentsFailed) return "error";
    if (this.consents === null) return "loading";
    if (!isGranted(this.consents, "sync") || syncLock.error?.code === "consent_required") return "syncOff";
    if (syncLock.status === "unlocking") return "unlocking";
    if (syncLock.status !== "unlocked") return "locked";
    if (this.decodeFailed) return "decodeFailed";
    if (this.synced === null) return "decoding";
    return "ready";
  });

  async loadConsents(): Promise<void> {
    this.consentsFailed = false;
    this.consents = null;
    try {
      this.consents = (await api<{ consents: Consent[] }>("/consents")).consents;
    } catch (e) {
      if (!(e instanceof ApiError && e.kind === "unauthorized")) this.consentsFailed = true;
    }
  }

  /** Machine names are a nicety: without them (consent off, or the list failed) ids stand in. */
  async loadMachines(): Promise<void> {
    this.machinesFailed = false;
    if (!this.consents || !isGranted(this.consents, "machines")) {
      this.machines = [];
      return;
    }
    try {
      this.machines = (await api<{ machines: HaryxMachine[] }>("/machines")).machines;
    } catch (e) {
      if (!(e instanceof ApiError && e.kind === "unauthorized")) this.machinesFailed = true;
    }
  }

  setConsents(next: Consent[]): void {
    this.consents = next;
  }

  /** Decodes the rows whenever the lock opens with new ones; drops them when it closes. */
  track(): () => void {
    return $effect.root(() => {
      $effect(() => {
        const rows = syncLock.rows;
        const key = syncLock.key;
        this.synced = null;
        this.decodeFailed = false;
        if (syncLock.status !== "unlocked" || !key) return;
        let live = true;
        decodeSynced(key, rows).then(
          (s) => {
            if (live) this.synced = s;
          },
          () => {
            if (live) this.decodeFailed = true;
          },
        );
        return () => {
          live = false;
        };
      });
    });
  }
}

export const vault = new Vault();

/** A machine's name from the registry, else a short id. */
export function machineName(id: string, machines: readonly HaryxMachine[]): string {
  return machines.find((m) => m.machineId === id)?.name ?? `Machine ${id.slice(0, 8)}`;
}
