// Build-time config. Production defaults point at WeCoAp SSO; dev overrides via VITE_SSO_* to reach
// the wecoap-test realm.
export interface SsoConfig {
  url: string;
  realm: string;
  clientId: string;
}

export function readConfig(env: Record<string, string | undefined> = import.meta.env): SsoConfig {
  return {
    url: (env.VITE_SSO_URL || "https://sso.wecoap.com").replace(/\/+$/, ""),
    realm: env.VITE_SSO_REALM || "wecoap",
    clientId: env.VITE_SSO_CLIENT || "haryx-platform",
  };
}

export const config = readConfig();

/** Delete account, password, linked identities and sessions live on the WeCoAp account page (A10). */
export const accountConsoleUrl = (c: SsoConfig = config) => `${c.url}/realms/${c.realm}/account/`;
