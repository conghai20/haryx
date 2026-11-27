import { describe, expect, it, vi } from "vitest";
import { ApiError, createApi } from "./api";

const reply = (status: number, body?: unknown) => new Response(body === undefined ? null : JSON.stringify(body), { status });
const make = (res: Response | Error, onUnauthorized = vi.fn()) => {
  const fetch = vi.fn(async () => {
    if (res instanceof Error) throw res;
    return res;
  });
  return { fetch, onUnauthorized, request: createApi({ getToken: async () => "tok", onUnauthorized, fetch: fetch as unknown as typeof globalThis.fetch }) };
};

describe("api client", () => {
  it("sends the bearer token to /api/v1 and parses JSON", async () => {
    const { request, fetch } = make(reply(200, { ok: 1 }));
    expect(await request("/machines")).toEqual({ ok: 1 });
    const [url, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/api/v1/machines");
    expect(new Headers(init.headers).get("Authorization")).toBe("Bearer tok");
  });
  it("returns undefined on 204", async () => {
    expect(await make(reply(204)).request("/x", { method: "DELETE" })).toBeUndefined();
  });
  it.each([
    [403, { error: "consent_required" }, "consent_required"],
    [403, { error: "nope" }, "forbidden"],
    [404, {}, "not_found"],
    [409, {}, "conflict"],
    [412, {}, "conflict"],
    [500, "garbage", "server"],
  ])("maps %i to %s", async (status, body, kind) => {
    await expect(make(reply(status, body)).request("/x")).rejects.toMatchObject({ kind, status });
  });
  it("401 is unauthorized and notifies the shell", async () => {
    const { request, onUnauthorized } = make(reply(401, { error: "unauthorized" }));
    await expect(request("/x")).rejects.toMatchObject({ kind: "unauthorized" });
    expect(onUnauthorized).toHaveBeenCalledOnce();
  });
  it("network failure is a typed error", async () => {
    const err = (await make(new TypeError("fetch failed")).request("/x").catch((e: unknown) => e)) as ApiError;
    expect(err).toBeInstanceOf(ApiError);
    expect(err.kind).toBe("network");
  });
});
