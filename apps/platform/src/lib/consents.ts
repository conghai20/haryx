import type { Consent, ConsentPurpose } from "./types";

/** The policy version the web grants. ponytail: constant until the realm publishes one to read. */
export const POLICY_VERSION = "2026-11-01";

export const PURPOSES: Record<ConsentPurpose, { title: string; grants: string; erases: string }> = {
  sync: {
    title: "Sync",
    grants: "Keeps your accounts, settings, usage and profile files in step across machines, end-to-end encrypted.",
    erases: "every synced account, setting, usage row and profile file on the server",
  },
  machines: {
    title: "Machines",
    grants: "Lists your machines (name, system, version, last seen) here and lets you sign them out.",
    erases: "your machine list",
  },
};

export type ConsentRequest = { method: "PUT"; path: string; body: string } | { method: "DELETE"; path: string };

/** The API call that turns a consent on or off. The server records the source (web) itself. */
export function consentRequest(purpose: ConsentPurpose, grant: boolean): ConsentRequest {
  const path = `/consents/${purpose}`;
  return grant ? { method: "PUT", path, body: JSON.stringify({ version: POLICY_VERSION }) } : { method: "DELETE", path };
}

export const withdrawWarning = (purpose: ConsentPurpose) =>
  `Withdrawing ${PURPOSES[purpose].title} erases ${PURPOSES[purpose].erases} now. This cannot be undone. Data on your machines stays.`;

export const isGranted = (consents: Consent[], purpose: ConsentPurpose) => consents.find((c) => c.purpose === purpose)?.granted ?? false;
