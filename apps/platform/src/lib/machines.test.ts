import { describe, expect, it, vi } from "vitest";
import { lastSeen, signOutMachine, signOutMessage, validateName } from "./machines";
import { consentRequest, withdrawWarning, POLICY_VERSION } from "./consents";

describe("validateName", () => {
  it("trims", () => expect(validateName("  Home PC ")).toEqual({ ok: true, name: "Home PC" }));
  it("rejects empty", () => expect(validateName("   ")).toMatchObject({ ok: false }));
  it("accepts 64, rejects 65 characters", () => {
    expect(validateName("a".repeat(64)).ok).toBe(true);
    expect(validateName("a".repeat(65)).ok).toBe(false);
  });
  it("counts characters, not UTF-16 units", () => expect(validateName("🐙".repeat(64)).ok).toBe(true));
});

describe("signOutMachine", () => {
  it("step 1 failing skips step 2", async () => {
    const record = vi.fn();
    const r = await signOutMachine({ endSession: async () => { throw new Error("x"); }, record });
    expect(r).toEqual({ ok: false, failedAt: "keycloak" });
    expect(record).not.toHaveBeenCalled();
  });
  it("step 2 failing is reported specifically, and retry skips step 1", async () => {
    const endSession = vi.fn(async () => {});
    const record = vi.fn().mockRejectedValueOnce(new Error("x")).mockResolvedValue(undefined);
    expect(await signOutMachine({ endSession, record })).toEqual({ ok: false, failedAt: "haryx" });
    expect(signOutMessage("haryx")).toMatch(/signed out of WeCoAp, but HaryX could not record/);
    expect(await signOutMachine({ endSession, record }, { skipKeycloak: true })).toEqual({ ok: true });
    expect(endSession).toHaveBeenCalledOnce();
  });
  it("runs both in order on success", async () => {
    const order: string[] = [];
    await signOutMachine({ endSession: async () => void order.push("1"), record: async () => void order.push("2") });
    expect(order).toEqual(["1", "2"]);
  });
});

describe("consentRequest", () => {
  it("grant is a PUT with the policy version", () =>
    expect(consentRequest("sync", true)).toEqual({ method: "PUT", path: "/consents/sync", body: JSON.stringify({ version: POLICY_VERSION }) }));
  it("withdraw is a DELETE", () => expect(consentRequest("machines", false)).toEqual({ method: "DELETE", path: "/consents/machines" }));
  it("says what is erased", () => {
    expect(withdrawWarning("sync")).toMatch(/every synced account, setting, usage row and profile file/);
    expect(withdrawWarning("machines")).toMatch(/machine list/);
  });
});

describe("lastSeen", () => {
  const now = Date.parse("2026-11-27T12:00:00Z");
  it("formats relative", () => {
    expect(lastSeen("2026-11-27T10:00:00Z", now)).toBe("2 hr ago");
    expect(lastSeen("2026-11-27T11:59:50Z", now)).toBe("just now");
    expect(lastSeen("2026-11-24T12:00:00Z", now)).toBe("3 days ago");
  });
});
