import { describe, expect, it } from "vitest";
import { accountConsoleUrl, readConfig } from "./config";

describe("readConfig", () => {
  it("defaults to production SSO", () => {
    expect(readConfig({})).toEqual({ url: "https://sso.wecoap.com", realm: "wecoap", clientId: "haryx-platform" });
  });
  it("takes overrides and trims trailing slashes", () => {
    const c = readConfig({ VITE_SSO_URL: "http://127.0.0.1:8080/", VITE_SSO_REALM: "wecoap-test", VITE_SSO_CLIENT: "x" });
    expect(c).toEqual({ url: "http://127.0.0.1:8080", realm: "wecoap-test", clientId: "x" });
    expect(accountConsoleUrl(c)).toBe("http://127.0.0.1:8080/realms/wecoap-test/account/");
  });
});
