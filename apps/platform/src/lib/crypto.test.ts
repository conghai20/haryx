// The fixture is written by the Rust CLI's own SyncKey (cli/crates/haryx-core/tests/browser_fixture.rs),
// which also checks on every run that it still opens in Rust.
import { generateIdentity } from "age-encryption";
import { describe, expect, it } from "vitest";
import fixture from "./fixtures/sync-fixture.json";
import { MAX_UNWRAP_LOG_N, KeyringError, scryptLogN, unwrapKeyring } from "./keyring";
import {
  CHECK_LABEL,
  accountLabel,
  importSyncKey,
  openDocument,
  openProfile,
  openRow,
  profileObjectKey,
  rowKey,
  type SyncRow,
} from "./rowcrypto";

const decode = (bytes: Uint8Array | null) => (bytes ? new TextDecoder().decode(bytes) : null);
const bytes = (b64: string) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
const asRow = (r: (typeof fixture.rows)[number]): SyncRow => ({ ...r, updatedAt: "2026-11-27T00:00:00Z" });
const wrappedOf = (body: string) => bytes((JSON.parse(body) as { wrapped: string }).wrapped);

async function refusal(body: string, passphrase: string): Promise<KeyringError> {
  const err = await unwrapKeyring(body, passphrase).then(
    () => null,
    (e: unknown) => e,
  );
  if (!(err instanceof KeyringError)) throw new Error(`expected a KeyringError, got ${String(err)}`);
  return err;
}

describe("keyring", () => {
  it("is wrapped at the pinned work factor", () => {
    expect(scryptLogN(wrappedOf(fixture.keyring))).toBe(18);
  });

  it("unwraps at log_n 18 to the CLI's key, its check verified", { timeout: 30_000 }, async () => {
    expect(await unwrapKeyring(fixture.keyring, fixture.passphrase)).toBe(fixture.secret);
  });

  it("refuses a wrong passphrase", { timeout: 30_000 }, async () => {
    const err = await refusal(fixture.keyring, "not the passphrase at all");
    expect(err.code).toBe("wrong_passphrase");
    expect(err.message).toBe("That is not your sync passphrase.");
  });

  it("refuses a keyring above log_n 20 before any scrypt work, with the CLI's message", async () => {
    expect(scryptLogN(wrappedOf(fixture.keyringLogN21))).toBe(21);
    const started = performance.now();
    const err = await refusal(fixture.keyringLogN21, fixture.passphrase);
    expect(performance.now() - started).toBeLessThan(200);
    expect(err.code).toBe("excessive_work");
    expect(err.message).toContain(`work factor of 21, more than HaryX opens (up to ${MAX_UNWRAP_LOG_N})`);
    expect(err.message).toContain("haryx sync passphrase");
  });

  it("calls a row that is not a keyring unreadable", async () => {
    expect((await refusal("not json", fixture.passphrase)).code).toBe("unreadable");
    expect((await refusal(JSON.stringify({ wrapped: "!!", check: "x" }), fixture.passphrase)).code).toBe("unreadable");
    expect((await refusal(fixture.rows[0]!.body, fixture.passphrase)).code).toBe("unreadable");
  });
});

describe("row keys", () => {
  it("match the CLI's HMAC for every label, ASCII or not", async () => {
    const key = await importSyncKey(fixture.secret);
    for (const { label, key: expected } of fixture.rowKeys) expect(await rowKey(key, label)).toBe(expected);
    expect(await rowKey(key, CHECK_LABEL)).toBe(fixture.check);
  });

  it("derive account labels the way the CLI does", async () => {
    expect(accountLabel("account", " You@Example.com ", "org-1111")).toBe("account:you@example.com|org-1111");
    expect(fixture.rowKeys.map((r) => r.label)).toContain(accountLabel("credential", "token@example.com", ""));
  });

  it("do not depend on how the secret is cased or padded", async () => {
    const a = await importSyncKey(fixture.secret);
    const b = await importSyncKey(`  ${fixture.secret.toLowerCase()}\n`);
    expect(await rowKey(b, "x")).toBe(await rowKey(a, "x"));
  });
});

describe("rows", () => {
  it("open byte for byte", async () => {
    const key = await importSyncKey(fixture.secret);
    for (const row of fixture.rows) expect(decode(await openRow(key, row.body))).toBe(row.plain);
  });

  it("are checked against the row they came from, as the CLI checks them", async () => {
    const key = await importSyncKey(fixture.secret);
    for (const row of fixture.rows.filter((r) => r.kind !== "usage")) {
      const opened = await openDocument(key, asRow(row));
      expect(opened.status).toBe(row.namesItself ? "doc" : "foreign");
      if (opened.status === "doc") expect(opened.doc).toEqual(JSON.parse(row.plain));
    }
  });

  it("carry a usage body that is the usage-sync contract", async () => {
    const key = await importSyncKey(fixture.secret);
    const usage = fixture.rows.find((r) => r.kind === "usage")!;
    const plain = JSON.parse(decode(await openRow(key, usage.body))!) as { machineId: string; month: string };
    expect(await rowKey(key, `usage:${plain.machineId}:${plain.month}`)).toBe(usage.key);
  });

  it("do not open with another key, or when they are not age at all", async () => {
    const stranger = await importSyncKey(await generateIdentity());
    const key = await importSyncKey(fixture.secret);
    expect(await openRow(stranger, fixture.rows[0]!.body)).toBeNull();
    expect(await openRow(key, "not base64 at all!")).toBeNull();
    expect((await openDocument(stranger, asRow(fixture.rows[0]!))).status).toBe("foreign");
  });
});

describe("profiles", () => {
  it("are named and opened as the CLI stores them", async () => {
    const key = await importSyncKey(fixture.secret);
    const { name, objectKey, file, plain } = fixture.profile;
    expect(await profileObjectKey(key, name)).toBe(objectKey);
    expect(decode(await openProfile(key, name, bytes(file)))).toBe(plain);
    expect(await openProfile(key, "other", bytes(file))).toBeNull();
  });
});
