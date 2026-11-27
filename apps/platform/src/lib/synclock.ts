// The app's one sync lock, shared by Accounts, Usage and Profiles.
import { api } from "./auth.svelte";
import type { SyncRow } from "./rowcrypto";
import { SyncLock, unwrapInWorker } from "./unlock.svelte";

export const syncLock = new SyncLock({
  fetchRows: () => api<SyncRow[]>("/sync/rows"),
  unwrap: unwrapInWorker,
  events: window,
  isVisible: () => document.visibilityState === "visible",
});
