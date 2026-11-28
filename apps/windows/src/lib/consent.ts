// The HaryX consents the app asks for, the WeCoAp pages it links to, and the words both use.
// The macOS app says the same (`apps/macos/Sources/HaryXCore/HaryXConsent.swift`); change the two
// together.

import type { Consents, ConsentPurpose, Whoami } from "./types";

/** The pages HaryX opens on the web; the backend allows exactly these (`updater::WECOAP_LINKS`). */
export const HARYX_WEB = "https://haryx.wecoap.com/platform/";
/** Where a WeCoAp ID, which is the HaryX account, is deleted (plan A10). */
export const WECOAP_ACCOUNT = "https://sso.wecoap.com/realms/wecoap/account/";
export const PRIVACY_POLICY = "https://www.wecoap.com/en/privacy-policy/";

/** The privacy policy version a grant records: the CLI's `POLICY_VERSION`. Bump them together. */
export const POLICY_VERSION = "2026-11-01";

/** Consent changes record where they were made. */
export const CONSENT_SOURCE = "windows";

export interface ConsentCopy {
  title: string;
  points: string[];
  agree: string;
  withdrawTitle: string;
  withdrawMessage: string;
  withdrawButton: string;
}

export const CONSENT_COPY: Record<ConsentPurpose, ConsentCopy> = {
  sync: {
    title: "Turn on sync?",
    points: [
      "HaryX uploads your settings, app preferences, account names and groups, API keys and tokens, setup profiles, and daily usage totals per account and model. Your Claude logins and your projects never leave this PC.",
      "Everything is end-to-end encrypted with your sync passphrase before it leaves this PC. HaryX cannot read it.",
      "The encrypted copy is stored on HaryX's servers in Vietnam.",
      "You can withdraw this consent at any time in HaryX Account. HaryX then stops sync on every machine and deletes its copy.",
    ],
    agree: "Agree and turn on sync",
    withdrawTitle: "Withdraw your consent to sync?",
    withdrawMessage:
      "Withdrawing Sync stops sync on every machine and deletes HaryX's server copy of your synced data: settings, accounts, credentials, profile files and usage. What is on each machine stays.",
    withdrawButton: "Withdraw and delete",
  },
  machines: {
    title: "Show this PC on HaryX web?",
    points: [
      "HaryX web lists this PC with its name, system, HaryX version and when it was last seen, so you can rename it or sign it out from your browser.",
      "This list is not encrypted: HaryX can read it. It is stored on HaryX's servers in Vietnam.",
      "Turn it off at any time. HaryX then deletes your machines from HaryX web. Sign-in and sync are not affected.",
    ],
    agree: "Agree and show",
    withdrawTitle: "Stop showing your machines on HaryX web?",
    withdrawMessage:
      "Withdrawing it deletes your machines from HaryX web: their names, systems, versions and when they were last seen. Sign-in and sync are not affected.",
    withdrawButton: "Stop showing",
  },
};

/** `haryx account consent grant|withdraw <purpose> --yes --source windows --json`. */
export function consentArgs(purpose: ConsentPurpose, grant: boolean): string[] {
  return ["account", "consent", grant ? "grant" : "withdraw", purpose, "--yes", "--source", CONSENT_SOURCE, "--json"];
}

/** Whether `purpose` is granted; null when the CLI could not say (offline, older CLI, signed out). */
export function consentGranted(who: Whoami | null, purpose: ConsentPurpose): boolean | null {
  if (!who?.signedIn || !who.consents) return null;
  return who.consents.find((c) => c.purpose === purpose)?.granted ?? false;
}

/** The same account with the consents a grant or withdraw answered. */
export function withConsents(who: Whoami, reply: Consents): Whoami {
  return {
    ...who,
    consents: reply.consents.map(({ purpose, granted, version }) => ({ purpose, granted, version })),
  };
}

/** The Usage pane's line on where usage data lives, which depends on the Sync consent. */
export function usageKept(syncing: boolean): string {
  return syncing
    ? "Daily totals per account and model sync end-to-end encrypted to your other machines and HaryX web; projects and sessions stay on this PC."
    : "It stays on this PC.";
}

export const DELETE_ACCOUNT_MESSAGE =
  "Your HaryX account is your WeCoAp ID, and it is deleted on the WeCoAp account page. To erase HaryX's data and keep your WeCoAp ID, withdraw both consents instead. Your Claude accounts on this PC are not touched.";
