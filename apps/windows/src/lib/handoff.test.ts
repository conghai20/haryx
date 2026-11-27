import { describe, expect, it } from "vitest";
import { bestTarget, stuckSessions } from "./handoff";
import type { Account, Session } from "./types";

const account = (number: number, five: number | null, extra: Partial<Account> = {}): Account => ({
  number,
  email: `a${number}@x.com`,
  active: number === 1,
  usageStatus: "ok",
  usage: { fiveHour: { pct: five }, sevenDay: { pct: 10 } },
  ...extra,
});

const session = (pid: number, accountNo: number | null, profile = "default"): Session => ({
  pid,
  account: accountNo,
  email: "",
  profile,
  sessionId: `s${pid}`,
  cwd: "C:\\w",
  startedAt: 0,
  kind: "interactive",
  entrypoint: "cli",
  status: "idle",
});

describe("session handoff", () => {
  const accounts = [account(1, 100), account(2, 40), account(3, 10), account(4, 5, { disabled: true })];

  it("finds `haryx run` sessions on exhausted accounts, never default-profile ones", () => {
    // A default-profile session picks up a switched account while it runs, so it is not stuck.
    const sessions = [session(10, 1, "session"), session(11, 2, "session"), session(12, 1)];
    const stuck = stuckSessions(sessions, accounts);
    expect(stuck.map((s) => [s.session.pid, s.account])).toEqual([[10, 1]]);
  });

  it("picks the usable account with the most headroom", () => {
    expect(bestTarget(accounts, 1)?.number).toBe(3);
    expect(bestTarget(accounts, 3)?.number).toBe(2);
    expect(bestTarget([account(1, 100), account(2, 100)], 1)).toBeNull();
  });

  it("prefers the source's group and leaves it only when none there can take the session", () => {
    const grouped = [
      account(1, 100, { group: "Work" }),
      account(2, 40, { group: "work" }),
      account(3, 5, { group: "Home" }),
      account(4, 1, { group: "Work", disabled: true }),
    ];
    expect(bestTarget(grouped, 1)?.number).toBe(2);
    const fullMate = grouped.map((a) => (a.number === 2 ? account(2, 100, { group: "Work" }) : a));
    expect(bestTarget(fullMate, 1)?.number).toBe(3);
    expect(bestTarget(grouped, null)?.number).toBe(3);
  });
});
