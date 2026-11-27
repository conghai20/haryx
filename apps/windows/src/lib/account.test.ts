import { describe, expect, it } from "vitest";
import whoami from "../../../../packages/contracts/haryx/whoami.json";
import consents from "../../../../packages/contracts/haryx/consents.json";
import machines from "../../../../packages/contracts/haryx/machines.json";
import usageSync from "../../../../packages/contracts/haryx/usage-sync.json";
import loginEvents from "../../../../packages/contracts/haryx/login-events.json";
import syncResult from "../../../../packages/contracts/haryx/sync-result.json";
import syncStatus from "../../../../packages/contracts/haryx/sync-status.json";
import {
  isFailure,
  overwriteLine,
  parseCli,
  passphraseProblem,
  providerName,
  syncProblem,
  syncedAgo,
} from "./account";
import type { AccountLoginEvent, Consents, HaryxMachines, RunResult, SyncResult, SyncStatus, UsageSync, Whoami } from "./types";

const run = (stdout: string, ok = true, message = ""): RunResult => ({ ok, status: ok ? 0 : 1, stdout, message });

describe("contract fixtures", () => {
  it("decodes whoami", () => {
    const w = parseCli<Whoami>(run(JSON.stringify(whoami)));
    expect(isFailure(w)).toBe(false);
    expect((w as Whoami).user?.email).toBe("you@example.com");
    expect(providerName((w as Whoami).user?.provider ?? "")).toBe("GitHub");
  });

  it("decodes whoami consents, the consents list, machines and a usage sync body", () => {
    const w = parseCli<Whoami>(run(JSON.stringify(whoami))) as Whoami;
    expect(w.consents?.map((c) => c.purpose)).toEqual(["sync", "machines"]);
    const c = parseCli<Consents>(run(JSON.stringify(consents))) as Consents;
    expect(c.consents[1]?.granted).toBe(false);
    expect(c.consents[1]?.history.map((e) => e.action)).toEqual(["grant", "withdraw"]);
    const m = parseCli<HaryxMachines>(run(JSON.stringify(machines))) as HaryxMachines;
    expect(m.machines.map((x) => x.isThisMachine)).toEqual([true, false]);
    const u = usageSync as UsageSync;
    expect(u.month).toBe("2026-11");
    expect(u.days[1]?.account).toBeNull();
    expect(JSON.stringify(u)).not.toMatch(/\/|project/);
  });

  it("decodes each login event in order", () => {
    const events = loginEvents as AccountLoginEvent[];
    expect(events.map((e) => e.event)).toEqual(["browser", "signed_in", "error"]);
    const [browser, signedIn] = events;
    expect(browser?.event === "browser" && browser.opened).toBe(true);
    expect(signedIn?.event === "signed_in" && signedIn.user.provider).toBe("github");
  });

  it("decodes a sync result and status", () => {
    const r = parseCli<SyncResult>(run(JSON.stringify(syncResult)));
    expect(isFailure(r)).toBe(false);
    if (!isFailure(r) && r.ok) {
      expect(r.changed).toEqual(["settings", "account"]);
      expect(r.otherMachineAccounts[0]?.alias).toBe("work");
      expect(overwriteLine(r.overwritten[0]!)).toBe("autoswitch.threshold: kept another machine's value, replaced 85");
    }
    const s = parseCli<SyncStatus>(run(JSON.stringify(syncStatus))) as SyncStatus;
    expect(s.signedIn && s.unlocked).toBe(true);
    expect(overwriteLine(s.overwritten[0]!)).toBe("you@example.com: alias: kept this PC's value, replaced old-name");
  });
});

describe("parseCli", () => {
  it("keeps a failure's code even on a non-zero exit", () => {
    const f = parseCli(run('{"ok":false,"code":"key_changed","message":"x"}\n', false));
    expect(isFailure(f) && f.code).toBe("key_changed");
  });

  it("turns non-JSON into a local failure with the CLI's message", () => {
    const f = parseCli(run("boom", false, "Error: boom"));
    expect(f).toEqual({ ok: false, code: "local", message: "Error: boom" });
    expect(parseCli(run("[1]"))).toMatchObject({ ok: false, code: "local" });
  });
});

describe("passphraseProblem", () => {
  it("needs 12 characters, typed twice the same", () => {
    expect(passphraseProblem("short", "short")).toMatch(/12 characters/);
    expect(passphraseProblem("twelve chars", "twelve charz")).toMatch(/do not match/);
    expect(passphraseProblem("twelve chars", "twelve chars")).toBeNull();
  });
});

describe("syncProblem", () => {
  const f = (code: string, message = "") => syncProblem({ ok: false, code, message });

  it("asks to unlock for locked, key_changed and a wrong passphrase", () => {
    for (const code of ["locked", "key_changed", "wrong_passphrase"]) expect(f(code).unlock).toBe(true);
    expect(f("key_changed").text).toMatch(/changed on another machine/);
  });

  it("is quiet about network and server trouble", () => {
    expect(f("network").text).toBe("Sync unavailable, will retry.");
    expect(f("server")).toEqual({ text: "Sync unavailable, will retry.", unlock: false, signedOut: false });
  });

  it("shows sign-in when the session is gone, and the CLI's words otherwise", () => {
    expect(f("signed_out").signedOut).toBe(true);
    expect(f("not_signed_in").signedOut).toBe(true);
    expect(f("busy").unlock).toBe(false);
    expect(f("invalid", "Bad row").text).toBe("Bad row");
  });
});

describe("syncedAgo", () => {
  const now = Date.parse("2026-11-25T10:00:00Z");
  it("reads relative times", () => {
    expect(syncedAgo(null, now)).toBe("Never synced");
    expect(syncedAgo("garbage", now)).toBe("Never synced");
    expect(syncedAgo("2026-11-25T09:59:30Z", now)).toBe("Synced just now");
    expect(syncedAgo("2026-11-25T09:45:00Z", now)).toBe("Synced 15 min ago");
    expect(syncedAgo("2026-11-25T07:00:00Z", now)).toBe("Synced 3 h ago");
    expect(syncedAgo("2026-11-20T07:00:00Z", now)).toMatch(/^Synced /);
  });
});
