// Decoding against the fixture the Rust CLI writes (cli/crates/haryx-core/tests/browser_fixture.rs).
// `usageTrusted` is what Rust's own `sync::usage::read_all` accepted from those rows.
import { describe, expect, it } from "vitest";
import fixture from "./fixtures/sync-fixture.json";
import { importSyncKey, rowKey, type SyncRow } from "./rowcrypto";
import { decodeSynced, parseAccount, parseMascotStyles, parseProfileIndex, parseUsage, usageLabel } from "./synced";
import { aggregate, machinesByAccount } from "./usagestats";
import { fileName, joinProfiles, openDownloaded, size } from "./profiles";
import contract from "../../../../packages/contracts/haryx/usage-sync.json";

const rows: SyncRow[] = fixture.rows.map((r) => ({ kind: r.kind, key: r.key, body: r.body, version: r.version, updatedAt: "2026-11-27T00:00:00Z" }));
const bytes = (b64: string) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));

describe("decodeSynced over the CLI's rows", () => {
  it("trusts exactly the usage rows Rust's read_all trusts", async () => {
    const key = await importSyncKey(fixture.secret);
    const synced = await decodeSynced(key, rows);
    const keys = await Promise.all(synced.usage.map((u) => rowKey(key, usageLabel(u.machineId, u.month))));
    expect(keys).toEqual(fixture.usageTrusted);
    expect(synced.usage[0]).toMatchObject({ machineId: expect.any(String), month: "2026-11" });
  });

  it("reads accounts that name their own row, never the moved one, never a credential", async () => {
    const key = await importSyncKey(fixture.secret);
    const synced = await decodeSynced(key, rows);
    expect(synced.accounts).toEqual([
      { email: "you@example.com", organizationUuid: "org-1111", organizationName: "Nguyễn's Org", alias: "Work", priority: 1, disabled: false, threshold: null },
    ]);
    expect(JSON.stringify(synced)).not.toContain("sk-ant");
    expect(synced.skipped).toBeGreaterThan(0);
  });

  it("reads mascot styles and the profile index", async () => {
    const key = await importSyncKey(fixture.secret);
    const synced = await decodeSynced(key, rows);
    expect(synced.mascotStyles).toEqual({ "you@example.com|org-1111": { species: "cat", hue: 0.55 } });
    expect(synced.profiles).toEqual([{ objectKey: fixture.profile.objectKey, name: "work", bytes: fixture.profile.plain.length, pushedAt: 1795600800 }]);
  });

  it("adds both machines' rows for the same day, account and model", async () => {
    const key = await importSyncKey(fixture.secret);
    const { usage } = await decodeSynced(key, rows);
    const [you] = aggregate(usage, "2026-11");
    expect(you!.account).toBe("you@example.com");
    expect(you!.machineIds).toHaveLength(2);
    const opus = you!.days.find((d) => d.day === "2026-11-24")!.models.find((m) => m.model === "claude-opus-4-5")!;
    expect(opus.replies).toBe(150);
    expect(opus.input).toBe(3000);
    expect(opus.cacheWrite).toBe(1_550_000);
    expect(machinesByAccount(usage).get("you@example.com")).toHaveLength(2);
  });

  it("trusts nothing with a stranger's key", async () => {
    const { generateIdentity } = await import("age-encryption");
    const synced = await decodeSynced(await importSyncKey(await generateIdentity()), rows);
    expect(synced).toMatchObject({ accounts: [], usage: [], profiles: [], mascotStyles: {} });
  });
});

describe("parseUsage", () => {
  it("reads the contract", () => expect(parseUsage(contract)).toEqual(contract));
  it("refuses another schema, a usage-share row, and a bad count anywhere", () => {
    expect(parseUsage({ ...contract, schema: 2 })).toBeNull();
    expect(parseUsage({ email: "you@example.com", org: "", fetchedBy: "m" })).toBeNull();
    expect(parseUsage({ ...contract, days: [{ ...contract.days[0], output: -1 }] })).toBeNull();
    expect(parseUsage({ ...contract, days: [{ ...contract.days[0], input: 1.5 }] })).toBeNull();
    expect(parseUsage({ ...contract, quota: [{ at: 1, account: null, fiveHourPct: "x", sevenDayPct: null }] })).toBeNull();
  });
  it("takes a missing account or percent as null, as serde does", () => {
    const { account: _a, ...day } = contract.days[0]!;
    const u = parseUsage({ ...contract, days: [day], quota: [{ at: 5 }] })!;
    expect(u.days[0]!.account).toBeNull();
    expect(u.quota[0]).toEqual({ at: 5, account: null, fiveHourPct: null, sevenDayPct: null });
  });
});

describe("value parsers", () => {
  it("account: settings with odd types fall back", () => {
    expect(parseAccount({ email: " a@b.c ", priority: "1", disabled: "yes", threshold: 80, alias: "  " })).toEqual({
      email: "a@b.c", organizationUuid: "", organizationName: "", alias: null, priority: null, disabled: false, threshold: 80,
    });
    expect(parseAccount({ alias: "x" })).toBeNull();
  });
  it("mascot styles: bad entries dropped, out-of-range hue cleared", () => {
    expect(parseMascotStyles({ mascotStyles: { a: { species: "fox", hue: 3 }, b: { hue: 1 }, c: "cat" } })).toEqual({ a: { species: "fox", hue: null } });
    expect(parseMascotStyles({})).toEqual({});
  });
  it("profile index: malformed entries dropped, sorted by name", () => {
    expect(parseProfileIndex({ k2: { name: "b", bytes: 1, pushedAt: 2 }, k1: { name: "a", bytes: 3, pushedAt: 4 }, k3: { name: "c" } }).map((p) => p.name)).toEqual(["a", "b"]);
  });
});

describe("aggregate", () => {
  const body = (machineId: string, days: object[], quota: object[] = []) =>
    parseUsage({ schema: 1, machineId, month: "2026-11", updatedAt: "", days, quota })!;
  const day = (d: string, account: string | null, model: string, input: number) =>
    ({ day: d, account, model, replies: 1, input, output: 0, cacheRead: 0, cacheWrite5m: 0, cacheWrite1h: 0 });
  const noon = (d: string) => Math.floor(new Date(`${d}T12:00:00`).getTime() / 1000);

  it("groups per account per day, newest first, models by size, email case folded", () => {
    const out = aggregate(
      [
        body("m1", [day("2026-11-01", "A@x.com", "opus", 10), day("2026-11-02", "a@x.com", "sonnet", 5), day("2026-11-02", "a@x.com", "opus", 50)]),
        body("m2", [day("2026-11-02", "a@x.com", "opus", 7), day("2026-11-02", null, "haiku", 1000)]),
      ],
      "2026-11",
    );
    expect(out.map((a) => a.account)).toEqual(["a@x.com", null]);
    const a = out[0]!;
    expect(a.total).toBe(72);
    expect(a.days.map((d) => d.day)).toEqual(["2026-11-02", "2026-11-01"]);
    expect(a.days[0]!.models.map((m) => [m.model, m.total])).toEqual([["opus", 57], ["sonnet", 5]]);
    expect(a.machineIds).toEqual(["m1", "m2"]);
  });

  it("keeps the day's highest readings and the newest one, across machines", () => {
    const out = aggregate(
      [
        body("m1", [], [{ at: noon("2026-11-03"), account: "a@x.com", fiveHourPct: 40, sevenDayPct: 10 }]),
        body("m2", [], [
          { at: noon("2026-11-03") + 60, account: "a@x.com", fiveHourPct: 20, sevenDayPct: 11 },
          { at: noon("2026-10-31"), account: "a@x.com", fiveHourPct: 99, sevenDayPct: 99 },
        ]),
      ],
      "2026-11",
    );
    const a = out[0]!;
    expect(a.days).toHaveLength(1);
    expect(a.days[0]).toMatchObject({ day: "2026-11-03", fiveHourPct: 40, sevenDayPct: 11, total: 0 });
    expect(a.latest).toEqual({ at: noon("2026-11-03") + 60, fiveHourPct: 20, sevenDayPct: 11 });
  });

  it("leaves other months out", () => {
    expect(aggregate([body("m1", [day("2026-11-01", "a@x.com", "opus", 1)])], "2026-10")).toEqual([]);
  });
});

describe("profiles", () => {
  it("joins the index to the server's files and opens the download as the CLI does", async () => {
    const key = await importSyncKey(fixture.secret);
    const { objectKey, name, file, plain } = fixture.profile;
    const index = [
      { objectKey, name, bytes: 31, pushedAt: 1 },
      { objectKey, name: "pointed-at-work", bytes: 1, pushedAt: 1 },
    ];
    const server = [
      { name: objectKey, bytes: 400, updatedAt: "2026-11-27T00:00:00Z" },
      { name: "ff", bytes: 9, updatedAt: "2026-11-27T00:00:00Z" },
    ];
    const { items, unnamed } = await joinProfiles(key, index, server);
    expect(items.map((i) => [i.name, i.storedBytes])).toEqual([["work", 400]]);
    expect(unnamed).toBe(1);
    expect(new TextDecoder().decode(await openDownloaded(key, name, bytes(file)))).toBe(plain);
    await expect(openDownloaded(key, "other", bytes(file))).rejects.toThrow(/does not open/);
  });

  it("names the download as a file the CLI imports", () => {
    expect(fileName("work")).toBe("work.haryx-profile");
    expect(fileName("../a/b\u0000")).toBe("__a_b_.haryx-profile");
    expect(size(812)).toBe("812 B");
    expect(size(48_213)).toBe("47 KB");
    expect(size(1_258_291)).toBe("1.2 MB");
  });
});
