import { describe, expect, it } from "vitest";
import whoami from "../../../../packages/contracts/haryx/whoami.json";
import consents from "../../../../packages/contracts/haryx/consents.json";
import { SIGN_IN_PROVIDERS } from "./account";
import {
  CONSENT_COPY,
  HARYX_WEB,
  POLICY_VERSION,
  PRIVACY_POLICY,
  WECOAP_ACCOUNT,
  consentArgs,
  consentGranted,
  usageKept,
  withConsents,
} from "./consent";
import type { Consents, Whoami } from "./types";

describe("consentGranted", () => {
  it("reads whoami's consents", () => {
    const w = whoami as Whoami;
    expect(consentGranted(w, "sync")).toBe(true);
    expect(consentGranted(w, "machines")).toBe(false);
  });

  it("is unknown, never false, without consents or signed out", () => {
    const user = { id: "1", email: "a@b.c", provider: "email" };
    expect(consentGranted({ signedIn: true, user }, "sync")).toBeNull();
    expect(consentGranted({ signedIn: false, user: null }, "sync")).toBeNull();
    expect(consentGranted(null, "sync")).toBeNull();
    const partial: Whoami = { signedIn: true, user, consents: [{ purpose: "sync", granted: true, version: "x" }] };
    expect(consentGranted(partial, "machines")).toBe(false);
  });
});

describe("withConsents", () => {
  it("replaces the consents with a grant or withdraw reply and keeps the user", () => {
    const w = whoami as Whoami;
    const updated = withConsents(w, consents as Consents);
    expect(updated.user).toEqual(w.user);
    expect(updated.consents).toEqual([
      { purpose: "sync", granted: true, version: "2026-11-01" },
      { purpose: "machines", granted: false, version: "2026-11-01" },
    ]);
    expect(w.consents?.[1]?.version).toBeNull();
  });
});

describe("consentArgs", () => {
  it("names the source and confirms for the user, who already saw the dialog", () => {
    expect(consentArgs("sync", true)).toEqual(["account", "consent", "grant", "sync", "--yes", "--source", "windows", "--json"]);
    expect(consentArgs("machines", false)).toEqual([
      "account", "consent", "withdraw", "machines", "--yes", "--source", "windows", "--json",
    ]);
  });
});

describe("copy", () => {
  it("names what each consent covers and what withdrawing erases", () => {
    expect(CONSENT_COPY.sync.points.join(" ")).toMatch(/end-to-end encrypted/);
    expect(CONSENT_COPY.sync.points.join(" ")).toMatch(/Vietnam/);
    expect(CONSENT_COPY.machines.title).toBe("Show this PC on HaryX web?");
    expect(CONSENT_COPY.sync.withdrawMessage).toMatch(/deletes HaryX's server copy/);
    expect(CONSENT_COPY.machines.withdrawMessage).toMatch(/deletes your machines from HaryX web/);
    expect(POLICY_VERSION).toBe("2026-11-01");
  });

  it("links only the pages the backend allows", () => {
    expect([HARYX_WEB, WECOAP_ACCOUNT, PRIVACY_POLICY]).toEqual([
      "https://haryx.wecoap.com/platform/",
      "https://sso.wecoap.com/realms/wecoap/account/",
      "https://www.wecoap.com/en/privacy-policy/",
    ]);
  });

  it("offers four ways to sign in", () => {
    expect(SIGN_IN_PROVIDERS.map((p) => p.id)).toEqual(["github", "google", "apple", "email"]);
    expect(SIGN_IN_PROVIDERS.map((p) => p.menu)).toEqual([
      "Continue with GitHub",
      "Continue with Google",
      "Continue with Apple",
      "Sign in with email",
    ]);
  });

  it("says usage stays on this PC only while it does not sync", () => {
    expect(usageKept(false)).toBe("It stays on this PC.");
    expect(usageKept(true)).toMatch(/end-to-end encrypted to your other machines and HaryX web/);
  });
});
